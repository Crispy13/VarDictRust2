#!/usr/bin/env bash
# sync_sweep_cache.sh — skeleton for syncing sweep fixtures with a remote cache
#
# Modes:
#   upload    Push local testdata/fixtures/... to remote cache (S3/LFS/etc.)
#   download  Pull remote cache into testdata/fixtures/...
#   verify    Compare local manifest hashes against remote (no writes)
#
# Backend: TBD. This skeleton stubs S3 and Git LFS and exits non-zero until a
# backend is wired in. The decision is tracked as an open question in
# docs/parity-scope.md §Non-goals / follow-ups. Add real logic once the
# project commits to a backend (expected inputs: bucket/endpoint URL,
# auth method, prefix convention).
#
# Layout contract (fixed regardless of backend):
#   <remote-root>/
#     manifest.json                  single source of truth (git-tracked locally)
#     <config>/<tag>/*.jsonl.zst     per-preset-per-tag golden shards
#
# Invocation:
#   bash scripts/sync_sweep_cache.sh upload
#   bash scripts/sync_sweep_cache.sh download --config T1-01 --tag hg002
#   bash scripts/sync_sweep_cache.sh verify

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE_ROOT="${REPO_ROOT}/testdata/fixtures/sweep"
MANIFEST="${CACHE_ROOT}/manifest.json"

# --- Backend stubs -----------------------------------------------------------
# Set SYNC_BACKEND to "s3" or "lfs" to pick a stub. Leave empty to get the
# "not configured" error. Reads from env: SYNC_BACKEND, SYNC_REMOTE, AWS_* (s3).

SYNC_BACKEND="${SYNC_BACKEND:-}"
SYNC_REMOTE="${SYNC_REMOTE:-}"

die() { echo "[sync_sweep_cache] $*" >&2; exit 1; }
log() { echo "[sync_sweep_cache] $*"; }

require_backend() {
  [[ -n "$SYNC_BACKEND" ]] || die "SYNC_BACKEND not set. Supported values: s3, lfs. Skeleton is stubbed; wire a real backend before use."
  [[ -n "$SYNC_REMOTE" ]]  || die "SYNC_REMOTE not set (e.g., s3://bucket/path or https://lfs.host/repo)."
}

s3_upload()   { die "s3 backend not implemented. Wire aws s3 sync or mc mirror here."; }
s3_download() { die "s3 backend not implemented. Wire aws s3 sync or mc mirror here."; }
s3_verify()   { die "s3 backend not implemented. Wire object-HEAD + etag comparison here."; }

lfs_upload()   { die "lfs backend not implemented. Wire git lfs push here."; }
lfs_download() { die "lfs backend not implemented. Wire git lfs pull --include here."; }
lfs_verify()   { die "lfs backend not implemented. Wire git lfs ls-files --json comparison here."; }

# --- Mode dispatch -----------------------------------------------------------

mode="${1:-}"
shift || true

case "$mode" in
  upload|download|verify)
    require_backend
    [[ -d "$CACHE_ROOT" ]] || die "cache root missing: $CACHE_ROOT"
    [[ -f "$MANIFEST" ]]   || die "manifest missing: $MANIFEST (generate via gen_e2e_sweep_golden.sh first)"
    log "mode=$mode backend=$SYNC_BACKEND remote=$SYNC_REMOTE"
    case "$SYNC_BACKEND" in
      s3)  "s3_${mode}" "$@" ;;
      lfs) "lfs_${mode}" "$@" ;;
      *)   die "unknown backend: $SYNC_BACKEND (want: s3, lfs)" ;;
    esac
    ;;
  -h|--help|"")
    sed -n '1,30p' "${BASH_SOURCE[0]}"
    exit 0
    ;;
  *)
    die "unknown mode: $mode (want: upload, download, verify)"
    ;;
esac
