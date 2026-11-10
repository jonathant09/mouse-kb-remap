#!/usr/bin/env bash
# The body of a GitHub release: the version's CHANGELOG.md section, then what
# each download is.
#
#   scripts/release-notes.sh 0.3.0 > notes.md
#
# Used by .github/workflows/build.yml when it publishes a release.
set -euo pipefail
version="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"

section=$(awk -v v="## $version" '$0 == v {on=1; next} /^## / {on=0} on' "$root/CHANGELOG.md")
if [ -z "$(echo "$section" | tr -d '[:space:]')" ]; then
  echo "CHANGELOG.md has no '## $version' section" >&2
  exit 1
fi

echo "$section" | sed -e '/./,$!d'
cat <<EOF

## Download

| File | |
| --- | --- |
| \`mouse-kb-remap-$version-windows-x64.zip\` | Unzip anywhere (for example your Documents folder) and run \`mouse-kb-remap.exe\` |
| \`mouse-kb-remap.exe\` | The same app as a single file |

Windows 10 and 11, nothing else to install. Windows SmartScreen may warn the first time
because the app isn't code-signed: click **More info → Run anyway**.

Full history: [CHANGELOG.md](https://github.com/jonathant09/mouse-kb-remap/blob/main/CHANGELOG.md)
EOF
