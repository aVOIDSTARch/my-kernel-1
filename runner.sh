#!/usr/bin/env bash
# Dispatches to run.sh (cargo run) or test-runner.sh (cargo test).
# cargo test places binaries under target/.../deps/; cargo run does not.
if [[ "$1" == *"/deps/"* ]]; then
    exec ./test-runner.sh "$@"
else
    exec ./run.sh "$@"
fi
