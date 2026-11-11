#!/bin/sh
# Runs a Windows program under Wine: the runner for the x86_64-pc-windows-gnu target
# (CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=scripts/wine-run.sh), so `cargo test` for that
# target runs the test binaries, and through them groovyfmt.exe, under Wine. WINE is the
# wine binary (default: wine, else Debian's /usr/lib/wine/wine64); WINEPREFIX defaults to
# ~/.wine-gf. Wine's own debug output is off. Wine maps Windows file names to Linux ones
# through the locale's charset, so without a UTF-8 locale it can't create `é.groovy`:
# C.UTF-8 unless the locale is already UTF-8.
set -eu
wine=${WINE:-}
if [ -z "$wine" ]; then
  if command -v wine > /dev/null; then wine=wine; else wine=/usr/lib/wine/wine64; fi
fi
case ${LC_ALL:-${LC_CTYPE:-${LANG:-}}} in
  *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*) ;;
  *) export LC_ALL=C.UTF-8 ;;
esac
WINEPREFIX=${WINEPREFIX:-$HOME/.wine-gf} WINEDEBUG=${WINEDEBUG:--all} exec "$wine" "$@"
