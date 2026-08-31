# Vendored JS

- `mermaid.min.js` — mermaid.js browser build, version 11.17.2. Vendored
  (not a Cargo/npm dependency) because this crate targets wasm32 with no JS
  package manager in the build; served from the daemon's own embedded
  bundle, no CDN call, per this project's local-first stance. Fetched from
  `https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.min.js`
  (~3.5 MB), confirmed to attach itself as `globalThis["mermaid"]`. To
  update: download the new release's `mermaid.min.js` browser build and
  replace this file, then update this version note.
