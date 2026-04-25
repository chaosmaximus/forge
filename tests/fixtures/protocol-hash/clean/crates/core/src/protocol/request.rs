// Synthetic Request enum for protocol-hash fixture tests.
pub enum Request {
    Health,
    Recall { query: String },
}
