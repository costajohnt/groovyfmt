# Changelog

The plugin's version follows groovyfmt's: version X.Y.Z downloads groovyfmt X.Y.Z when it finds no groovyfmt installed. The groovyfmt changes are in its [changelog](https://github.com/costajohnt/groovyfmt/blob/main/CHANGELOG.md).

## 0.14.0 (2026-11-13)

- First release. Starts `groovyfmt --lsp` through LSP4IJ for Groovy, Gradle and Jenkinsfiles: parse errors and lint violations as you type, quick fixes and fix all, and Code | Reformat with groovyfmt. Reformat Code uses groovyfmt when "Use groovyfmt for Reformat Code" is on. Finds groovyfmt through its path settings, the project's node_modules and PATH, or downloads the matching release after asking, verified against the SHA-256 checksums the plugin release carries. In an untrusted project nothing from the project decides which binary runs.
