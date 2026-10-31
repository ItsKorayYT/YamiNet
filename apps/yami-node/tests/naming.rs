use std::{
    io::{self, Read, Write},
    sync::atomic::{AtomicUsize, Ordering},
};
struct Memory {
    incoming: io::Cursor<Vec<u8>>,
    written: Vec<u8>,
}
impl Read for Memory {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.incoming.read(b)
    }
}
impl Write for Memory {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.written.extend(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl ynp::Connection for Memory {
    fn protection(&self) -> ynp::Protection {
        ynp::Protection::ExperimentalPlaintext
    }
}
struct Documents(AtomicUsize);
impl yami_node::Documents for Documents {
    fn lookup(&self, _: &str) -> io::Result<Option<ynp::DocumentData>> {
        Ok(None)
    }
    fn naming(&self, _: &[u8]) -> io::Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(b"{}".to_vec())
    }
}
#[test]
fn plaintext_naming_is_rejected_before_any_service_mutation() {
    let mut bytes = Vec::new();
    ynp::write_frame(
        &mut bytes,
        &ynp::Frame {
            request_id: 4,
            message: ynp::Message::NamingRequest {
                bytes: b"{}".to_vec(),
            },
        },
    )
    .unwrap();
    let mut connection = Memory {
        incoming: io::Cursor::new(bytes),
        written: Vec::new(),
    };
    let documents = Documents(AtomicUsize::new(0));
    assert_eq!(
        yami_node::serve_connection(&mut connection, &documents)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(documents.0.load(Ordering::SeqCst), 0);
    assert!(connection.written.is_empty());
}
