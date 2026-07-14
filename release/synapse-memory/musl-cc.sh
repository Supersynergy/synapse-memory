#!/usr/bin/env sh
set -eu

: "${SYNAPSE_MUSL_CC:?SYNAPSE_MUSL_CC is required}"

for arg in "$@"; do
  case "$arg" in
    *sqlite-vec.c)
      exec "$SYNAPSE_MUSL_CC" -D_GNU_SOURCE -include sys/types.h "$@"
      ;;
  esac
done

exec "$SYNAPSE_MUSL_CC" "$@"
