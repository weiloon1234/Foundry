#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${FOUNDRY_BUILD_METRICS_TARGET_DIR:-${project_root}/target/build-metrics}"

case "${target_dir}" in
  "${project_root}"/target/*) ;;
  *)
    echo "FOUNDRY_BUILD_METRICS_TARGET_DIR must be inside ${project_root}/target" >&2
    exit 1
    ;;
esac

cargo clean --target-dir "${target_dir}" >/dev/null

started_at="$(date +%s)"
CARGO_TARGET_DIR="${target_dir}" cargo build \
  --manifest-path "${project_root}/Cargo.toml" \
  --release \
  -p foundry
finished_at="$(date +%s)"

artifact_path="$(find "${target_dir}/release/deps" -maxdepth 1 -type f -name 'libfoundry-*.rlib' -print | sort | head -n 1)"
if [[ -z "${artifact_path}" ]]; then
  echo "release build did not produce a Foundry rlib" >&2
  exit 1
fi

if stat -f '%z' "${artifact_path}" >/dev/null 2>&1; then
  artifact_bytes="$(stat -f '%z' "${artifact_path}")"
else
  artifact_bytes="$(stat -c '%s' "${artifact_path}")"
fi

target_kib="$(du -sk "${target_dir}" | awk '{print $1}')"
dependency_packages="$(
  cargo tree \
    --manifest-path "${project_root}/Cargo.toml" \
    -p foundry \
    --edges normal \
    --prefix none \
    --format '{p}' \
    | sort -u \
    | wc -l \
    | tr -d ' '
)"

cat <<METRICS
# Foundry clean release-build metrics

- Measured at (UTC): $(date -u '+%Y-%m-%dT%H:%M:%SZ')
- Host: $(uname -sm)
- Rust: $(rustc --version)
- Clean release build: $((finished_at - started_at)) seconds
- Foundry rlib: ${artifact_bytes} bytes
- Dedicated Cargo target: ${target_kib} KiB
- Normal dependency packages: ${dependency_packages}
- Target directory: ${target_dir}
METRICS
