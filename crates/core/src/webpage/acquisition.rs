use super::RelFlags;
use std::collections::HashMap;
use url::Url;
pub(crate) struct Script {
    pub(crate) attributes: HashMap<String, String>,
    pub(crate) content: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    pub source: Url,
    pub destination: Url,
    pub rel: RelFlags,
    pub text: String,
}

pub type Meta = HashMap<String, String>;
