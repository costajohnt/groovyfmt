# Changelog

The extension's version follows groovyfmt's: version X.Y.Z downloads groovyfmt X.Y.Z when it finds no groovyfmt installed. The groovyfmt changes are in its [changelog](https://github.com/costajohnt/groovyfmt/blob/main/CHANGELOG.md).

## Unreleased

- First release. Starts `groovyfmt --lsp` for Groovy, Gradle and Jenkinsfiles: formatting of documents and selections, parse errors and lint violations as you type, quick fixes and fix all (`source.fixAll.groovyfmt`). Finds groovyfmt through `groovyfmt.path`, the workspace's `node_modules` and `PATH`, or downloads the matching release after asking, verified against the SHA-256 checksums the extension release carries.
