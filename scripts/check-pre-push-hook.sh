#!/usr/bin/env bash
set -euo pipefail

hook=".githooks/pre-push"

if ! grep -q 'task fmt:check' "$hook"; then
  echo "pre-push hook must keep the non-mutating format check" >&2
  exit 1
fi

default_section="$(sed '/PRE_PUSH_FULL/,$d' "$hook")"
full_section="$(sed -n '/PRE_PUSH_FULL/,/fi/p' "$hook")"

if grep -Eq '^[[:space:]]*task([[:space:]]+--[[:alnum:]-]+)*[[:space:]]+(lint|ci)([[:space:]]|$)' <<<"$default_section"; then
  echo "pre-push hook must not run artifact-building tasks by default" >&2
  exit 1
fi

if ! grep -q 'PRE_PUSH_FULL' <<<"$full_section" || ! grep -Eq '^[[:space:]]*task([[:space:]]+--[[:alnum:]-]+)*[[:space:]]+ci([[:space:]]|$)' <<<"$full_section"; then
  echo "pre-push hook must gate task ci behind PRE_PUSH_FULL" >&2
  exit 1
fi
