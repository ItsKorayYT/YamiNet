//! Small in-memory document node. No discovery, persistence or background fetches.
use std::{collections::BTreeMap, io};
use ynp::{DocumentData, Frame, Incoming, Message};

#[derive(Default)]
pub struct Store(BTreeMap<String, DocumentData>);
impl Store {
    pub fn insert(&mut self, document: DocumentData) -> io::Result<()> {
        // Validate all wire bounds before serving, without buffering a frame.
        ynp::write_frame(
            &mut io::sink(),
            &Frame {
                request_id: 0,
                message: Message::Document(document.clone()),
            },
        )?;
        self.0.insert(document.key.clone(), document);
        Ok(())
    }
}
/// One bounded request per connection. A bad client affects only its connection.
pub fn serve_once(listener: &impl Incoming, store: &Store) -> io::Result<()> {
    let mut connection = listener.accept()?;
    let request = ynp::read_frame(&mut *connection)?;
    let Message::Get { key } = request.message else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "node accepts only Get requests",
        ));
    };
    let message = match store.0.get(&key) {
        Some(doc) => Message::Document(doc.clone()),
        None => Message::NotFound { key },
    };
    ynp::write_frame(
        &mut *connection,
        &Frame {
            request_id: request.request_id,
            message,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn store_rejects_unservable_documents() {
        assert!(Store::default()
            .insert(DocumentData {
                key: "../bad".into(),
                source: String::new(),
                style: String::new()
            })
            .is_err());
    }
}
