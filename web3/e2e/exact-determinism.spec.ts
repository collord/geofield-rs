// Stage 0 exact-corefiner determinism gate (docs/12 §2.3, §5).
//
// The exact-arithmetic tri-tri intersection (crates/geofield/src/exact_arr) constructs its segment
// endpoints as EXACT rationals and snapshots them to f64 with a single correctly-rounded step — so
// the output bits come from rounding an exact value, not accumulated f64 ops, and must be identical
// on native and wasm. The native side pins the digest as a golden constant
// (`exact_arr::tri_tri::spike_digest_is_deterministic_and_pinned`). This spec runs the SAME code in
// the browser's wasm (V8) and asserts the same digest — the cross-target half the `#[cfg(test)]`
// suite cannot run (no `wasm-bindgen-test` harness; memory `geofield-no-wasm-cargo-test-harness`).
//
// PREREQUISITE: the engine wasm must be built `--features exact_arr` for the `exact_spike_digest_hex`
// export (and thus `window.__exactSpikeDigest`) to exist — use `npm run test:e2e:exact`, which runs
// `npm run wasm:engine:exact` first. With the default build (no feature) the hook is absent and this
// spec SKIPS, keeping the default suite green and the production bundle unchanged.
//
// NOTE: the determinism premise was first proven out-of-band by running the very same wasm32
// artifact under node/V8 (docs/12); this spec is the permanent, canonical CI form of that check —
// browsers and node share V8, and wasm f64 is spec-deterministic across all conforming runtimes.

import { test, expect } from "@playwright/test";

// Mirror of the native golden constant in crates/geofield/src/exact_arr/tri_tri.rs. If the native
// fixture/digest changes, update both in lockstep (the native test prints the new value).
const NATIVE_GOLDEN = "0E4B9D0B1F846BD1";

test("exact tri-tri spike digest is bit-identical native↔wasm", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => typeof (window as any).__exactSpikeDigest === "function");

  const digest = await page.evaluate(async () => {
    return await (window as any).__exactSpikeDigest();
  });

  test.skip(
    digest === undefined,
    "engine wasm built without --features exact_arr (run `npm run test:e2e:exact`)",
  );

  expect(
    digest,
    `browser wasm digest 0x${digest} != native golden 0x${NATIVE_GOLDEN} — exact construction diverged across targets`,
  ).toBe(NATIVE_GOLDEN);
});
