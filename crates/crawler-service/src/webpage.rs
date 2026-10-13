#[path = "../../core/src/webpage/acquisition.rs"]
pub mod acquisition;
#[path = "../../core/src/webpage/adservers.rs"]
pub mod adservers;
#[path = "../../core/src/webpage/html/mod.rs"]
pub mod html;
#[path = "../../core/src/webpage/just_text.rs"]
pub mod just_text;
#[path = "../../core/src/webpage/schema_org/mod.rs"]
pub mod schema_org;
#[path = "../../core/src/webpage/url_ext.rs"]
pub mod url_ext;
use acquisition::Script;
pub use acquisition::{Link, Meta};
pub use html::{links::RelFlags, Html};
