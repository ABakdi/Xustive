#!/usr/bin/env bash
# Download the model weights.
#
# Weights are not in the repository: they are gigabytes, they change independently of the code,
# and their licences are their own. This script fetches them into `models/`, which is gitignored.
#
# Every model here is Apache-2.0 and self-hostable. See `crates/xustive-ml/src/registry.rs` for
# the registry these entries mirror — the two must agree, and the check at the end enforces it.
set -euo pipefail

cd "$(dirname "$0")/.."
DIR="${XUSTIVE_MODEL_DIR:-models}"
mkdir -p "$DIR"

# id | file | repo | approximate MiB
MODELS=(
  "qwen2.5-3b-instruct-q4|qwen2.5-3b-instruct-q4_k_m.gguf|Qwen/Qwen2.5-3B-Instruct-GGUF|2007"
  "qwen2.5-1.5b-instruct-q4|qwen2.5-1.5b-instruct-q4_k_m.gguf|Qwen/Qwen2.5-1.5B-Instruct-GGUF|1070"
)

WANTED="${1:-default}"

fetch() {
  local file="$1" repo="$2" mib="$3"
  local dest="$DIR/$file"

  if [ -s "$dest" ]; then
    echo "  ✓ $file already present ($(du -h "$dest" | cut -f1))"
    return
  fi

  echo "  ↓ $file (~${mib} MiB) from $repo"
  # --fail so an HTML error page never lands on disk pretending to be a model, and a partial
  # download is removed rather than left to fail confusingly at load time.
  if ! curl -fL --progress-bar -o "$dest.part" \
      "https://huggingface.co/$repo/resolve/main/$file?download=true"; then
    rm -f "$dest.part"
    echo "  ✗ failed to download $file" >&2
    return 1
  fi
  mv "$dest.part" "$dest"

  # GGUF files begin with the magic bytes "GGUF". A truncated or redirected download does not.
  if [ "$(head -c 4 "$dest")" != "GGUF" ]; then
    rm -f "$dest"
    echo "  ✗ $file is not a GGUF file; removed" >&2
    return 1
  fi
}

echo "Fetching models into $DIR/"
for entry in "${MODELS[@]}"; do
  IFS='|' read -r id file repo mib <<< "$entry"
  case "$WANTED" in
    all) ;;
    default) [ "$id" = "qwen2.5-3b-instruct-q4" ] || continue ;;
    "$id") ;;
    *) continue ;;
  esac
  fetch "$file" "$repo" "$mib"
done

echo
echo "Present:"
ls -lh "$DIR" 2>/dev/null | tail -n +2 | awk '{printf "  %-45s %s\n", $9, $5}'
echo
echo "The summariser picks one at startup. Check /admin to see which."
