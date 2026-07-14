#!/usr/bin/env sh
set -eu

prefix="${SYNAPSE_PREFIX:-$HOME/.local}"
db="${SYNAPSE_DB:-$HOME/.synapse/brain.db}"
purge=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --purge-data) purge=1 ;;
    --prefix) prefix="$2"; shift ;;
    --db) db="$2"; shift ;;
    -h|--help)
      printf '%s\n' "usage: uninstall.sh [--purge-data] [--prefix DIR] [--db FILE]"
      exit 0
      ;;
    *) printf 'error: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
  shift
done

rm -f "$prefix/bin/synx"
if [ "$purge" = 1 ]; then
  rm -f "$db" "$db-wal" "$db-shm"
  printf 'removed binary and data=%s\n' "$db"
else
  printf 'removed binary; preserved data=%s\n' "$db"
fi
