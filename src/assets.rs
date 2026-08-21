//! Embedded static assets (rust-embed style via include_bytes! at build time).

static FILES: &[(&str, &str, &[u8])] = &[
    (
        "style.css",
        "text/css",
        include_bytes!("../static/style.css"),
    ),
    (
        "app.js",
        "application/javascript",
        include_bytes!("../static/app.js"),
    ),
    (
        "htmx.min.js",
        "application/javascript",
        include_bytes!("../static/htmx.min.js"),
    ),
];

pub fn get(path: &str) -> Option<(&'static str, &'static [u8])> {
    FILES
        .iter()
        .find(|(name, _, _)| *name == path)
        .map(|(_n, mime, bytes)| (*mime, *bytes))
}
