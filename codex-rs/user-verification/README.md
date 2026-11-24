# Native hardware test

`tests/hardware.rs` exercises the macOS provider with a real persisted Secure
Enclave key: create and reload it, verify a signature, cancel an authentication
request, and sign a new challenge with the same key. It is ignored by default.

Run on a physical Touch ID Mac with fingerprints enrolled and the lid open. Use
an existing **isolated development app** with a valid provisioning profile and
Keychain Sharing entitlements, plus its Apple Development signing identity.
The test does not use a Wallet account or submit a purchase. Run only one copy at
a time; it reserves the `codex-user-verification-hardware-test` key namespace and
removes that namespace's key before and after the test.

From the repository root, set `TEMPLATE_APP` to that signed development `.app`
and `SIGNING_IDENTITY` to its signing identity, then run in Bash. Signing the
runner is insufficient: the test executable itself needs the profile and
entitlements.

```bash
set -euo pipefail
: "${TEMPLATE_APP:?Set the path to the isolated signed development app}"
: "${SIGNING_IDENTITY:?Set its Apple Development identity}"
run_dir=$(mktemp -d)
app="$run_dir/HardwareTest.app"
(cd codex-rs && cargo build --locked -p codex-user-verification --test hardware \
  --message-format=json) > "$run_dir/build.jsonl"
binary=$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "hardware" and .executable != null) | .executable' "$run_dir/build.jsonl")
mkdir -p "$app/Contents/MacOS"
cp "$binary" "$app/Contents/MacOS/hardware"
cp "$TEMPLATE_APP/Contents/Info.plist" "$app/Contents/Info.plist"
cp "$TEMPLATE_APP/Contents/embedded.provisionprofile" "$app/Contents/embedded.provisionprofile"
/usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable hardware' "$app/Contents/Info.plist"
codesign -d --xml --entitlements - "$TEMPLATE_APP" > "$run_dir/entitlements.plist"
codesign --force --options runtime --entitlements "$run_dir/entitlements.plist" \
  --sign "$SIGNING_IDENTITY" "$app"
codesign --verify --strict "$app"
echo "Test bundle: $app"
"$app/Contents/MacOS/hardware" --ignored --exact \
  persisted_key_signing_cancellation_and_retry --nocapture --test-threads=1
```

Use Touch ID for the first prompt, then remove your finger. Leave the
**cancellation** prompt untouched and observe that it appears and closes on its
own. Authenticate the **retry signing** prompt once. Passing assertions verify
both signatures, the cancellation result, and key removal; confirming visible
prompt dismissal is the operator's part of the test. An early interruption fails
the test rather than counting as cancellation.

If cancellation has not returned ten seconds after it is issued, the watchdog
exits with a failure instead of hanging. After a forced exit, stop any remaining
test process and remove the reserved key with the same signed bundle:

```bash
CODEX_HARDWARE_TEST_CLEANUP=1 "$app/Contents/MacOS/hardware" --ignored --exact \
  persisted_key_signing_cancellation_and_retry --nocapture --test-threads=1
```

This test covers the native provider. It does not exercise app-server routing,
UI state, closed-lid recovery, or hosted Wallet completion.
