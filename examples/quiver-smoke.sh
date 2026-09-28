#!/usr/bin/env bash
# examples/quiver-smoke.sh — exercise the Quiver JSON-RPC surface without Wox.
#
# Usage:
#   examples/quiver-smoke.sh                       # auto-detect target/release/quiver(.exe)
#   WOX_QUIVER_EXE=path/to/quiver examples/quiver-smoke.sh
#
# Reads examples/ShellCommands.json as the catalog (via WOX_DIRECTORY_USER_DATA).
# Prints a green/red summary and exits non-zero on the first failure.
#
# This is a smoke harness — keep it simple and shell-portable (bash 4+).
# It is NOT part of the crate's test target; `cargo test` covers logic, this
# covers the wiring: binary exists, stdin/stdout flow, stub renderer, catalog
# loader, fuzzy ranking, capture gate, and the action path (a `run` action
# that really spawns a command, plus the clipboard hook).
#
# Dependencies: bash 4+, python3 (the JSON field extractor), and the
# commands the demo entries themselves use (`ip`, `sed`, `touch`).

set -uo pipefail

here="$(cd "$(dirname -- "$0")" && pwd)"

# ---- locate the binary ----
# Host-native candidates only: a cross-compiled Windows PE executed through
# WSL interop cannot read the POSIX path in WOX_DIRECTORY_USER_DATA, so it
# would fail the catalog checks for the wrong reason.
exe="${WOX_QUIVER_EXE:-}"
if [[ -z "$exe" ]]; then
    for candidate in \
        "$here/../target/release/quiver" \
        "$here/../target/release/quiver.exe" \
        "$(command -v quiver 2>/dev/null || true)"
    do
        [[ -x "$candidate" ]] && exe="$candidate" && break
    done
fi

if [[ -z "$exe" || ! -x "$exe" ]]; then
    printf 'quiver-smoke: cannot find quiver binary (set WOX_QUIVER_EXE or run `make` first)\n' >&2
    exit 1
fi

printf 'quiver-smoke: using binary %s\n' "$exe"

# ---- dependency gate: fail loud, not with confusing want/got diffs ----
if ! command -v python3 >/dev/null 2>&1; then
    printf 'quiver-smoke: python3 is required (JSON field extractor) but not on PATH\n' >&2
    exit 1
fi

# ---- temp files: mktemp + one trap, nothing left behind on Ctrl-C ----
tmp_log="$(mktemp)"
tmp_out="$(mktemp)"
cleanup() { rm -f "$tmp_log" "$tmp_out"; }
trap cleanup EXIT INT TERM

# json_get — extract a field from a flat JSON object on stdin.
# Args: <dotted.field.path>. Implemented on python3 (the harness's one
json_get() {
    local field="$1"
    python3 -c "
import json, sys
try:
    obj = json.loads(sys.stdin.read())
except Exception:
    sys.exit(1)
v = obj
for part in '$field'.split('.'):
    if isinstance(v, dict):
        v = v.get(part)
    elif isinstance(v, list) and part.lstrip('-').isdigit():
        i = int(part)
        v = v[i] if -len(v) <= i < len(v) else None
    else:
        v = None
    if v is None:
        sys.exit(0)
if isinstance(v, (dict, list)):
    print(json.dumps(v))
else:
    print(v)
" 2>/dev/null
}

# ---- helpers ----
pass=0
fail=0
fail_msgs=()

expect_eq() {
    local label="$1" want="$2" got="$3"
    if [[ "$want" == "$got" ]]; then
        printf '  ok   %s\n' "$label"
        (( pass += 1 ))
    else
        printf '  FAIL %s\n      want: %s\n      got:  %s\n' "$label" "$want" "$got"
        (( fail += 1 ))
        fail_msgs+=("$label")
    fi
}

# expect_contains <label> <substring> <haystack>
# Uses grep -F (fixed-string) against a temp file so multi-line haystacks
# are matched line-by-line. bash `[[ =~ ]]` was rejected because:
#   1. variable-interpolated patterns have brace-escape quirks;
#   2. `^` only matches start-of-string, not start-of-line.
expect_contains() {
    local label="$1" needle="$2" haystack="$3"
    printf '%s' "$haystack" >"$tmp_out"
    if grep -qF -- "$needle" "$tmp_out"; then
        printf '  ok   %s\n' "$label"
        (( pass += 1 ))
    else
        printf '  FAIL %s\n      missing: %s\n' "$label" "$needle"
        (( fail += 1 ))
        fail_msgs+=("$label")
    fi
}

# expect_matches <label> <ERE pattern> <haystack>
# Anchors are caller's responsibility (`^...`/`...$`). Patterns are passed
# to grep -E (POSIX ERE), which avoids bash `[[ =~ ]]` quirks around `{`.
expect_matches() {
    local label="$1" pattern="$2" haystack="$3"
    printf '%s' "$haystack" >"$tmp_out"
    if grep -qE -- "$pattern" "$tmp_out"; then
        printf '  ok   %s\n' "$label"
        (( pass += 1 ))
    else
        printf '  FAIL %s\n      pattern: %s\n' "$label" "$pattern"
        (( fail += 1 ))
        fail_msgs+=("$label")
    fi
}

# expect_absent <label> <substring> <haystack>
expect_absent() {
    local label="$1" needle="$2" haystack="$3"
    printf '%s' "$haystack" >"$tmp_out"
    if grep -qF -- "$needle" "$tmp_out"; then
        printf '  FAIL %s\n      unexpected: %s\n' "$label" "$needle"
        (( fail += 1 ))
        fail_msgs+=("$label")
    else
        printf '  ok   %s\n' "$label"
        (( pass += 1 ))
    fi
}

# call <method> <search>
# Sends a query JSON-RPC request, prints the raw response on stdout.
call() {
    local method="$1" search="$2"
    WOX_DIRECTORY_USER_DATA="$here" \
        "$exe" <<JSON
{"jsonrpc":"2.0","id":1,"method":"$method","params":{"triggerKeyword":"qv","search":"$search"}}
JSON
}

# ---- 1. stub renderer: both flavours produce parseable headers ----
printf '\n[1] stub renderer\n'
stub_posix="$("$exe" --stub posix)"
stub_win="$("$exe" --stub windows)"

expect_matches 'posix stub: starts with shebang naming quiver' \
    '^#!.*quiver' "$stub_posix"
expect_matches 'posix stub: line 2 opens the JSON comment block' \
    '^# \{' "$stub_posix"
expect_contains 'posix stub: declares Runtime=SCRIPT' \
    '"Runtime": "SCRIPT"' "$stub_posix"
expect_contains 'posix stub: declares TriggerKeywords' \
    '"TriggerKeywords"' "$stub_posix"
expect_contains 'posix stub: lists `quiver` keyword' \
    '"quiver"' "$stub_posix"
expect_contains 'posix stub: lists `qv` keyword' \
    '"qv"' "$stub_posix"

expect_matches 'windows stub: line 1 opens the JSON comment block directly' \
    '^# \{' "$stub_win"
expect_contains 'windows stub: declares Runtime=SCRIPT' \
    '"Runtime": "SCRIPT"' "$stub_win"

# ---- 2. JSON-RPC envelope shape ----
printf '\n[2] JSON-RPC envelope\n'
resp="$(call query 'echo hello')"
expect_eq 'response is JSON-RPC 2.0' '2.0' "$(printf '%s' "$resp" | json_get jsonrpc)"
# JSON-RPC 2.0 §4: the echoed id keeps the request's type — the harness
# sends an integer, so `1` (not `"1"`, not `""`) must come back.
expect_contains 'response echoes the numeric id verbatim' \
    '"id":1' "$resp"
expect_contains 'response has a result.items array' \
    '"result":{"items"' "$resp"
expect_absent 'no error key on success' \
    '"error"' "$resp"

# An unknown method is -32601, not a fabricated empty result.
resp="$(WOX_DIRECTORY_USER_DATA="$here" "$exe" <<<'{"jsonrpc":"2.0","id":7,"method":"nope","params":{}}')"
expect_eq 'unknown method yields -32601' \
    '-32601' "$(printf '%s' "$resp" | json_get error.code)"

# ---- 3. exact-alias query: the capture contract, data included ----
printf '\n[3] exact-alias lookup + capture payload\n'
resp="$(call query 'now')"
expect_contains 'exact alias `now` returns its title' \
    '"title":"now"' "$resp"
expect_contains 'capture entry binds Enter to copy-to-clipboard' \
    '"id":"copy-to-clipboard"' "$resp"
expect_matches 'capture entry has an ISO-8601 clipboard payload' \
    '"data":"20[0-9]{2}-[0-9]{2}-[0-9]{2}T' "$resp"
expect_matches 'capture entry carries a markdown preview' \
    '"preview":\{"type":"markdown"' "$resp"

# ---- 4. fuzzy ranking: prefix "ip" ranks the ip alias, with its data ----
printf '\n[4] fuzzy ranking\n'
resp="$(call query 'ip')"
expect_contains 'fuzzy `ip` returns an item titled `ip`' \
    '"title":"ip"' "$resp"
if command -v ip >/dev/null 2>&1; then
    want_ip="$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]\+\) .*/\1/p')"
    if [[ -n "$want_ip" ]]; then
        expect_eq 'capture data is the primary IPv4 (computed the same way)' \
            "$want_ip" "$(printf '%s' "$resp" | json_get result.items.0.actions.0.data)"
    fi
else
    printf '  skip capture-data check: `ip` not on PATH\n'
fi

# ---- 5. empty query: items array is well-formed, no error ----
printf '\n[5] empty query\n'
resp="$(call query '')"
expect_contains 'empty query: still a well-formed items array' \
    '"items"' "$resp"
expect_absent 'empty query has no error' \
    '"error"' "$resp"

# ---- 6. parse error path: malformed JSON returns a JSON-RPC -32700 error ----
printf '\n[6] malformed input\n'
WOX_DIRECTORY_USER_DATA="$here" "$exe" <<< 'this is not json' >"$tmp_out" 2>/dev/null
err="$(cat "$tmp_out")"
expect_eq 'malformed JSON yields parse-error code -32700' \
    '-32700' "$(printf '%s' "$err" | json_get error.code)"
expect_eq 'parse error carries id:null (spec shape for an unreadable id)' \
    'null' "$(printf '%s' "$err" | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin).get("id")))')"

# ---- 7. structured logging: QUIVER_LOG drives stderr emission ----
# Verify the level gates:
#   * unset            → error still emits; warn/info/debug do not
#   * QUIVER_LOG=warn  → warn/error emit; info/debug do not
#   * QUIVER_LOG=info  → info+warn+error emit
#   * QUIVER_LOG=debug → all four emit (the catalog-load debug line is a
#     stable signal: it precedes the `catalog loaded` info line)
#   * garbage value    → the error floor holds (regression: a typo'd
#     directive used to silence even ERROR output)
printf '\n[7] structured logging\n'
# 7a. default (no QUIVER_LOG): probe with a *successful* query so the
#     absence assertions can actually fail (a parse-error probe emits no
#     INFO/WARN at any level).
WOX_DIRECTORY_USER_DATA="$here" "$exe" \
    <<<'{"jsonrpc":"2.0","id":1,"method":"query","params":{"search":"now"}}' \
    >/dev/null 2>"$tmp_log"
log_default="$(cat "$tmp_log")"
expect_contains 'default level: errors always loud (parse error probe)' \
    ' ERROR ' "$(WOX_DIRECTORY_USER_DATA="$here" "$exe" <<<'not json' 2>&1 >/dev/null)"
expect_absent 'default level: no INFO lines on a successful query' \
    ' INFO ' "$log_default"
expect_absent 'default level: no WARN lines on a successful query' \
    ' WARN ' "$log_default"

# 7b. QUIVER_LOG=info → INFO lines emitted; DEBUG not yet.
WOX_DIRECTORY_USER_DATA="$here" QUIVER_LOG=info "$exe" \
    <<<'{"jsonrpc":"2.0","id":1,"method":"query","params":{"triggerKeyword":"qv","search":"ip"}}' \
    >/dev/null 2>"$tmp_log"
log_info="$(cat "$tmp_log")"
expect_contains 'QUIVER_LOG=info: catalog loaded emits INFO' \
    ' INFO catalog loaded' "$log_info"
expect_absent 'QUIVER_LOG=info: no DEBUG lines (need QUIVER_LOG=debug)' \
    ' DEBUG ' "$log_info"

# 7c. QUIVER_LOG=debug → DEBUG + INFO both present.
WOX_DIRECTORY_USER_DATA="$here" QUIVER_LOG=debug "$exe" \
    <<<'{"jsonrpc":"2.0","id":1,"method":"query","params":{"triggerKeyword":"qv","search":"ip"}}' \
    >/dev/null 2>"$tmp_log"
log_debug="$(cat "$tmp_log")"
expect_contains 'QUIVER_LOG=debug: query received emits DEBUG' \
    ' DEBUG query received' "$log_debug"
expect_contains 'QUIVER_LOG=debug: catalog_load path emits DEBUG' \
    ' DEBUG catalog_load path' "$log_debug"

# 7d. QUIVER_LOG=warn + bogus --stub arg → fallback WARN line.
QUIVER_LOG=warn "$exe" --stub bogus >/dev/null 2>"$tmp_log"
log_warn="$(cat "$tmp_log")"
expect_contains 'QUIVER_LOG=warn: unknown --stub arg emits WARN with got' \
    ' WARN --stub received unknown layout' "$log_warn"
expect_contains 'QUIVER_LOG=warn: WARN line includes got="bogus"' \
    'got="bogus"' "$log_warn"

# 7e. garbage directive → the error floor holds. This is the regression
#     test for the silence bug: EnvFilter reads a bare word as a *target*,
#     so `QUIVER_LOG=not-a-level` alone used to swallow ERROR output.
QUIVER_LOG=not-a-level "$exe" <<<'not json' >/dev/null 2>"$tmp_log"
log_garbage="$(cat "$tmp_log")"
expect_contains 'garbage QUIVER_LOG still emits ERROR (error floor)' \
    ' ERROR json parse failed' "$log_garbage"

# ---- 8. action path: run really spawns; clipboard hook is a clean no-op ----
printf '\n[8] action path\n'
marker="$(mktemp -d)/ran"
resp="$(WOX_DIRECTORY_USER_DATA="$here" "$exe" <<JSON
{"jsonrpc":"2.0","id":1,"method":"action","params":{"id":"run","data":{"command":"touch $(printf '%s' "$marker" | sed 's/\\/\\\\/g')","interpreter":"sh","cwd":"/tmp"}}}
JSON
)"
expect_eq 'run action answers an empty result' \
    '{}' "$(printf '%s' "$resp" | json_get result)"
# The spawn is fire-and-forget; poll briefly for the side effect.
marker_seen=0
for _ in $(seq 1 50); do
    if [[ -e "$marker" ]]; then marker_seen=1; break; fi
    sleep 0.1
done
expect_eq 'run action really spawned the command (marker file)' \
    '1' "$marker_seen"

resp="$(WOX_DIRECTORY_USER_DATA="$here" "$exe" <<<'{"jsonrpc":"2.0","id":1,"method":"action","params":{"id":"copy-to-clipboard","data":{"command":"touch /definitely/not/this"}}}')"
expect_eq 'clipboard hook is a clean no-op (Wox already copied)' \
    '{}' "$(printf '%s' "$resp" | json_get result)"
expect_eq 'and it did not spawn the stale data command' \
    '0' "$([[ -e /definitely/not/this ]] && echo 1 || echo 0)"

# ---- 9. catalog-path override: QUIVER_PATH bypasses the directory ----
# Both variables named together so the precedence rule is observable: the
# file path wins over the directory variable. The probe also uses a
# catalog that contains *only* the `probe` alias, so a successful hit
# proves the file was loaded, not the directory fallback.
printf '\n[9] catalog-path override\n'
probe_dir="$(mktemp -d)"
probe_alias='pathoverride'
probe_catalog="$probe_dir/single.json"
cat > "$probe_catalog" <<JSON
{"version":1,"defaultInterpreter":"bash","defaultWorkingDirectory":"","commands":[
    {"alias":"$probe_alias","command":"echo path-override-ok"}
]}
JSON
# Both vars set — file path must win.
resp="$(WOX_DIRECTORY_USER_DATA="$here" QUIVER_PATH="$probe_catalog" \
    "$exe" <<<"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"search\":\"$probe_alias\"}}")"
expect_contains 'QUIVER_PATH wins over WOX_DIRECTORY_USER_DATA' \
    "\"title\":\"$probe_alias\"" "$resp"

# The probe alias must NOT come from the example catalog — set the file path
# to one whose contents do *not* include `now`, and prove `now` is no longer
# a hit. This is the strongest evidence the file path actually wins, not
# the directory fallback.
resp="$(WOX_DIRECTORY_USER_DATA="$here" QUIVER_PATH="$probe_catalog" \
    "$exe" <<<"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"search\":\"now\"}}")"
expect_absent 'file path actually used (not the directory fallback)' \
    '"title":"now"' "$resp"

# Empty QUIVER_PATH is treated as unset, falling back to the dir.
resp="$(WOX_DIRECTORY_USER_DATA="$here" QUIVER_PATH="" \
    "$exe" <<<"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"search\":\"now\"}}")"
expect_contains 'empty QUIVER_PATH falls back to WOX_DIRECTORY_USER_DATA' \
    '"title":"now"' "$resp"
rm -rf "$probe_dir"

# ---- summary ----
printf '\nquiver-smoke: %d passed, %d failed\n' "$pass" "$fail"
if (( fail > 0 )); then
    printf 'failed checks:\n'
    for m in "${fail_msgs[@]}"; do printf '  - %s\n' "$m"; done
    exit 1
fi
