// Same content as clean fixture — drift comes from plugin.json's stale hash.
pub enum Request {
    Health,
    Recall { query: String },
}
