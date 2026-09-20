#!/usr/bin/env bash
# Record a fixture's raw runner output into tests/output/ for the parser
# tests. Re-run when a runner changes its format.
#
#   scripts/record.sh go-basic          # writes tests/output/go-basic.json
#   scripts/record.sh cargo-basic       # writes tests/output/cargo-basic.txt
#
# The runner's exit code is expected to be non-zero (the fixtures fail on
# purpose) and is written to tests/output/<fixture>.code.
#
# Runners print absolute paths. The fixture's absolute directory is rewritten
# to /FIXTURE_ROOT so recordings are the same on every machine, and pytest's
# hostname attribute is blanked. Parser tests pass /FIXTURE_ROOT as the root
# for adapters that do not read files from it.
set -uo pipefail

fixture="${1:-}"
[[ -n "$fixture" ]] || { echo "usage: scripts/record.sh FIXTURE" >&2; exit 2; }
cd "$(dirname "$0")/.."
dir="fixtures/$fixture"
[[ -d "$dir" ]] || { echo "no such fixture: $dir" >&2; exit 1; }
mkdir -p tests/output
state="$(mktemp -d)"
trap 'rm -rf "$state"' EXIT
code=0

case "$fixture" in
  go-*)
    (cd "$dir" && go test -json ./...) > "tests/output/$fixture.json" 2> "tests/output/$fixture.stderr" || code=$?
    ;;
  cargo-*)
    (cd "$dir" && cargo test --no-fail-fast) > "tests/output/$fixture.txt" 2> "tests/output/$fixture.stderr" || code=$?
    ;;
  jest-*)
    (cd "$dir" && npx jest --json --outputFile="$state/jest.json" --testLocationInResults) > /dev/null 2> "tests/output/$fixture.stderr" || code=$?
    cp "$state/jest.json" "tests/output/$fixture.json"
    ;;
  vitest-*)
    (cd "$dir" && npx vitest run --reporter=json --outputFile="$state/vitest.json") > /dev/null 2> "tests/output/$fixture.stderr" || code=$?
    cp "$state/vitest.json" "tests/output/$fixture.json"
    ;;
  nodetest-*)
    (cd "$dir" && node --test --test-reporter=tap) > "tests/output/$fixture.tap" 2> "tests/output/$fixture.stderr" || code=$?
    ;;
  pytest-*)
    (cd "$dir" && pytest -o junit_family=xunit1 --junitxml="$state/junit.xml" -q) > "tests/output/$fixture.stdout" 2> "tests/output/$fixture.stderr" || code=$?
    cp "$state/junit.xml" "tests/output/$fixture.xml"
    ;;
  *)
    echo "no recorder for $fixture" >&2; exit 1
    ;;
esac
abs="$(cd "$dir" && pwd -P)"
for f in tests/output/"$fixture".*; do
  perl -pi -e 's{\Q'"$abs"'\E}{/FIXTURE_ROOT}g; s{hostname="[^"]*"}{hostname=""}g' "$f"
done
echo "$code" > "tests/output/$fixture.code"
# Empty stderr files carry nothing; drop them.
[[ -s "tests/output/$fixture.stderr" ]] || rm -f "tests/output/$fixture.stderr"
echo "recorded $fixture (exit $code)"
ls tests/output/"$fixture".*
