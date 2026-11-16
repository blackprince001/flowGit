#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
source_app="$repo_root/build/flowGit.app"
install_dir="${GITFLOW_INSTALL_DIR:-/Applications}"
destination="$install_dir/flowGit.app"

if [ "$(uname -s)" != "Darwin" ]; then
  printf 'flowGit can only be installed as an app on macOS.\n' >&2
  exit 1
fi

"$repo_root/scripts/build-app.sh"

if [ ! -d "$install_dir" ]; then
  if ! mkdir -p "$install_dir" 2>/dev/null; then
    printf 'Creating %s requires administrator access.\n' "$install_dir"
    sudo mkdir -p "$install_dir"
  fi
fi

if [ -w "$install_dir" ] && { [ ! -e "$destination" ] || [ -w "$destination" ]; }; then
  ditto "$source_app" "$destination"
else
  printf 'Installing to %s requires administrator access.\n' "$install_dir"
  sudo ditto "$source_app" "$destination"
fi

codesign --verify --deep --strict "$destination"
printf 'Installed %s\n' "$destination"
