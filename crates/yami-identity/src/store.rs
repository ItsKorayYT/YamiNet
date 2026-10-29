//! Persistent node identity in a directory:
//!
//! * `node.key`: secret, 72 bytes: magic `YNSK`, format version 1, algorithm 1
//!   (Ed25519), two zero bytes, the 32-byte secret seed, the 32-byte public key.
//! * `node.id`: public, the node ID followed by a newline.
//!
//! The stored public key and node ID are cross-checked on every load, so a
//! damaged or swapped file is reported instead of becoming a new identity.
use crate::{Error, NodeId, Result};
use ed25519_dalek::SigningKey;
use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub const KEY_FILE: &str = "node.key";
pub const ID_FILE: &str = "node.id";
const MAGIC: &[u8; 4] = b"YNSK";
const FORMAT_VERSION: u8 = 1;
const ALGORITHM_ED25519: u8 = 1;
const KEY_FILE_LEN: usize = 8 + 32 + 32;
const MAX_ID_FILE: usize = 128;

/// A node's key pair. The secret key is zeroized on drop, never formatted and
/// never leaves this type except into the owner-only key file.
pub struct NodeIdentity {
    signing: SigningKey,
    id: NodeId,
}
impl fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeIdentity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl NodeIdentity {
    /// A new random identity from the operating system's secure generator.
    pub fn generate() -> Result<Self> {
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(seed.as_mut()).map_err(|error| {
            Error::Io(io::Error::other(format!(
                "system random number generator failed: {error}"
            )))
        })?;
        Ok(Self::from_seed(&seed))
    }
    fn from_seed(seed: &[u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(seed);
        let id = NodeId::new(signing.verifying_key());
        Self { signing, id }
    }
    pub fn id(&self) -> &NodeId {
        &self.id
    }
    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.signing
    }

    /// Generate and persist a new identity. Never replaces existing files.
    pub fn create(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let (key_path, id_path) = (dir.join(KEY_FILE), dir.join(ID_FILE));
        for path in [&key_path, &id_path] {
            if path.symlink_metadata().is_ok() {
                return Err(Error::AlreadyExists(path.clone()));
            }
        }
        create_private_dir(dir)?;
        let identity = Self::generate()?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_FILE_LEN));
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&[FORMAT_VERSION, ALGORITHM_ED25519, 0, 0]);
        bytes.extend_from_slice(identity.signing.as_bytes());
        bytes.extend_from_slice(&identity.id.public_key());
        write_new(&key_path, &bytes)?;
        if let Err(error) = write_new(&id_path, format!("{}\n", identity.id).as_bytes()) {
            // Nothing has used the new key yet; do not leave half an identity.
            let _ = fs::remove_file(&key_path);
            return Err(error);
        }
        Ok(identity)
    }

    /// Load a persisted identity, verifying the key file and the node ID file.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let key_path = dir.join(KEY_FILE);
        let file = match File::open(&key_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Error::MissingKey(key_path))
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(Error::CorruptKey("not a regular file"));
        }
        check_private(&key_path, &metadata)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_FILE_LEN + 1));
        file.take(KEY_FILE_LEN as u64 + 1).read_to_end(&mut bytes)?;
        let identity = parse_key(&bytes)?;
        if let Some(recorded) = read_id(&dir.join(ID_FILE))? {
            if recorded != identity.id {
                return Err(Error::Mismatch {
                    expected: recorded,
                    found: identity.id,
                });
            }
        }
        Ok(identity)
    }

    /// Load the identity, or create one if the directory has none. Returns
    /// whether it was created. If only `node.id` exists the key was lost, and
    /// generating a different identity under the same name is refused.
    pub fn load_or_create(dir: impl AsRef<Path>) -> Result<(Self, bool)> {
        let dir = dir.as_ref();
        match Self::load(dir) {
            Err(Error::MissingKey(path)) => {
                if dir.join(ID_FILE).symlink_metadata().is_ok() {
                    return Err(Error::MissingKey(path));
                }
                Ok((Self::create(dir)?, true))
            }
            loaded => Ok((loaded?, false)),
        }
    }
}

fn parse_key(bytes: &[u8]) -> Result<NodeIdentity> {
    if bytes.len() != KEY_FILE_LEN {
        return Err(Error::CorruptKey("wrong length"));
    }
    if &bytes[..4] != MAGIC {
        return Err(Error::CorruptKey("not a YamiNet node key file"));
    }
    if bytes[4] != FORMAT_VERSION {
        return Err(Error::CorruptKey("unsupported key file version"));
    }
    if bytes[5] != ALGORITHM_ED25519 {
        return Err(Error::CorruptKey("unsupported key algorithm"));
    }
    if bytes[6..8] != [0, 0] {
        return Err(Error::CorruptKey("reserved bytes are not zero"));
    }
    let mut seed = Zeroizing::new([0; 32]);
    seed.copy_from_slice(&bytes[8..40]);
    let identity = NodeIdentity::from_seed(&seed);
    if identity.id.public_key() != bytes[40..] {
        return Err(Error::CorruptKey(
            "secret key does not match the stored public key",
        ));
    }
    Ok(identity)
}

fn read_id(path: &Path) -> Result<Option<NodeId>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut text = String::new();
    file.take(MAX_ID_FILE as u64)
        .read_to_string(&mut text)
        .map_err(|_| Error::InvalidNodeId("node.id is not UTF-8 text"))?;
    Ok(Some(text.trim_end_matches(['\r', '\n']).parse()?))
}

/// Where identities live by default: a per-user application data directory
/// outside any source checkout. `LOCALAPPDATA` is used on Windows because,
/// unlike `APPDATA`, it does not roam to other machines.
pub fn default_dir() -> Option<PathBuf> {
    let absolute = |name| {
        env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if cfg!(windows) {
        return absolute("LOCALAPPDATA").map(|base| base.join("YamiNet").join("identity"));
    }
    absolute("XDG_DATA_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local").join("share")))
        .map(|base| base.join("yaminet").join("identity"))
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Create a file that must not exist yet. On Unix it is owner-only from the
/// moment it exists. On Windows it inherits the directory's access control
/// list; this crate does not change ACLs.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => Error::AlreadyExists(path.into()),
        _ => error.into(),
    })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(unix)]
fn check_private(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::InsecurePermissions(path.into()));
    }
    Ok(())
}
#[cfg(not(unix))]
fn check_private(_: &Path, _: &fs::Metadata) -> Result<()> {
    Ok(())
}
