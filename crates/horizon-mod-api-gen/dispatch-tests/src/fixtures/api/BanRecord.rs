#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BanRecord {
    pub uuid: Option<u128>,
    pub name: Option<String>,
    pub ip: Option<String>,
    pub created_millis: i64,
    pub source: String,
    pub expires_millis: Option<i64>,
    pub reason: Option<String>,
}
