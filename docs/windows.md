# Windows

Tested on Windows Server 2022 (x86_64, NTFS) and under Wine. Not yet tested on a desktop Windows release or on ARM.

## Install

`groovyfmt-<version>-x86_64-pc-windows-gnu.zip` on each [release](https://github.com/costajohnt/groovyfmt/releases) holds `groovyfmt.exe`. It is one file that needs only Windows' own DLLs, so it runs on a Windows machine that has never had a compiler or runtime installed. It is cross-compiled on Linux with mingw-w64.

In PowerShell:

```powershell
gh release download --repo costajohnt/groovyfmt --pattern '*windows*'
(Get-FileHash groovyfmt-*.zip -Algorithm SHA256).Hash.ToLower()   # compare with the .sha256 file
Expand-Archive groovyfmt-*.zip .
```

Then put the folder holding `groovyfmt.exe` on `PATH`. `tar -xf` (the `tar.exe` Windows ships) unpacks the zip too.

The [GitHub Action](ci.md#github-actions), the npm package (`@groovyfmt/win32-x64`) and the [Gradle plugin](ci.md#gradle) install it on Windows x64 too. There is no Windows on ARM build.

## What was tested

The whole test suite passes on Windows Server 2022 and under Wine. Formatting gives files byte-identical to the Linux build's, on a 3,000-file sample of the corpus on Windows Server and on the whole 23,766-file corpus under Wine.

[Releasing](releasing.md#windows) describes how the Windows build is made and tested.

## What to expect

Each point was checked on Windows Server 2022 unless it says otherwise.

### Output and paths

- Output is the same as on Linux and macOS. Paths print with `/` in every format (text, `github`, JSON, SARIF, `--diff` headers).
- groovyfmt never turns `\n` into `\r\n`. CRLF files keep their line endings, and `--lsp` framing stays exact over pipes.
- Paths can be typed with `\` or `/`, with or without a drive letter.
- `exclude` patterns use `/` and are case-sensitive, as everywhere, so a config written on Linux works unchanged.

### Writing files

- Formatting in place writes a temporary file beside the original and renames it over it, as elsewhere, so a crash never leaves half a file.
- A read-only file is not formatted (exit 2).
- A file another program has open is formatted if that program opened it with delete sharing (`FILE_SHARE_DELETE`). Otherwise groovyfmt names the file, exits 2 and leaves it as it was.
- Before the rename, groovyfmt checks that the path still holds the file it read, by size, modification time and contents (Windows has no inode number std can read).
- The rewritten file gets the permissions (ACL) its folder gives new files, so a file with explicit permissions of its own loses them. There is no owner to keep.
- Hard links can't be seen, so formatting a hard-linked file replaces it and its other names keep the old text. On Linux and macOS they stay linked.

### Links and names

- Symlinks and junctions get the same rules as on Linux and macOS. They are not followed inside a walk, are followed when named, and are not written through when named and pointing outside the working directory (`groovyfmt: vendor: symlink target is outside the working directory, not formatted`, exit 2).
- `CON`, `NUL`, `COM1` and the other device names are never created or read. Naming one gives `no such file or directory` (exit 2).
- Windows drops a trailing dot or space from a name, so `A.groovy.` is `A.groovy`.
- Paths longer than 260 characters work, without the `LongPathsEnabled` setting.

### Shells

- Neither PowerShell nor `cmd` expands `*.groovy` for a program on Windows, and groovyfmt doesn't either. Name a directory (`groovyfmt --check .`), which is searched as on Linux, or let PowerShell expand the names (`groovyfmt --check (Get-ChildItem *.groovy).Name`).
- `--stdin` reads and writes bytes. Windows PowerShell 5.1 re-encodes what it pipes to a program (as ASCII by default, so `é` becomes `?`) and adds line endings, and its `>` writes UTF-16. Use `cmd`'s redirection, which passes bytes through: `cmd /c "groovyfmt --stdin < A.groovy > out.groovy"`. Editors and the `--lsp` server don't go through a shell and aren't affected.
- File names print as UTF-8. To read non-ASCII ones in a PowerShell 5.1 console, first run `[Console]::OutputEncoding = [Text.Encoding]::UTF8`.
- In PowerShell 5.1 a quoted argument can't end in `\` (`'my dir\'` reaches groovyfmt as `my dir"`). Write `'my dir'` or `'my dir/'`.
- `--changed-since` runs `git` from `PATH` (Git for Windows). Without it, it exits 2 with `running git: program not found`. Not yet tried with Git for Windows installed.

## Not checked yet

- desktop Windows 10 and 11
- Windows on ARM
- antivirus or indexer locks on a file being replaced
- PowerShell 7
- Git for Windows
