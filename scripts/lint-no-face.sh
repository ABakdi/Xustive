#!/usr/bin/env bash
# No face recognition, by construction (M10-T05.4, [[UI - Image Search]]).
#
# Image similarity is whole-image, never people. This fails the build on the commit that adds a
# face-detection or face-embedding dependency anywhere — the Rust lock file or any sidecar's
# requirements — so the rule cannot be crossed by accident.
set -uo pipefail
cd "$(dirname "$0")/.."
pattern='(insightface|facenet|face_recognition|face-recognition|deepface|retinaface|arcface|dlib|mediapipe|mtcnn|rustface|facial)'
hits=$( { grep -inE "$pattern" Cargo.lock services/*/requirements.txt 2>/dev/null || true; } )
if [ -n "$hits" ]; then
  echo "✗ face lint: a face-recognition dependency appeared:"; echo "$hits"; exit 1
fi
echo "✓ face lint: no face-detection or face-embedding library in any dependency list"
