/// Which browser is running, and therefore which transport answers DOM
/// questions and which browser-specific helpers apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFlavor {
    Chrome,
    Edge,
    Firefox,
}
