# Cross-PDF JavaScript navigation: ScriptSession migration plan

Status: implementation design; no runtime behavior changed yet.

## Baseline

This branch starts at upstream `main` after #7144. The upstream implementation of
`app.openDoc(cPath, oDoc)` in `crates/js/src/globals.rs` validates a local path,
queues `Request::OpenDoc(String)`, and returns `null`. It already rejects
unsupported filesystems and document contexts. Preserve these checks.

The earlier prototype lives on `feat/acrobat-open-doc-bridge` and is a
reference only, not a merge source. Its `crates/js/src/menu.rs` uses
`request_open_doc` and a Boa accessor proxy for `pageNum`, while its UI
implements optional remembered path grants. That code predates upstream's
per-document `ScriptSession` and must not be cherry-picked wholesale.

## Intended behavior

```javascript
var target = app.openDoc("./other.pdf", this);
target.pageNum = 42; // zero-based, deferred until approved/opened
```

1. Every `openDoc` call gets an independent, script-scoped request identity.
2. The returned object is a *restricted deferred handle*, not a real PDF
   document and not an authorization capability. Expose only `pageNum`.
3. The setter validates finite, non-negative page indices and records the
   destination on that exact request. Define an explicit integer conversion
   policy and test it. Page bounds are checked against the target PDF after
   opening, not against the source PDF.
4. All file access remains in the viewer's existing vetted opener, after
   authorization. JavaScript evaluation must never open/read the target file.
5. Existing tabs and in-flight opens should be reused; page navigation must
   be associated with the correct requested target and applied only after
   its successful open.
6. Requests from a closed source document must be discarded. Never grant
   access on a rejected or canceled request.
7. Preserve `ScriptSession` isolation and worker lifetime; do not reintroduce
   the previous independent interactive JavaScript worker.

## Security decision before enabling persistent consent

The prototype's remembered grant was keyed by absolute source path, SHA-256
of source bytes, and absolute target path. This does **not** bind the target
file's identity: replacing a file or retargeting a symlink can silently
change what a remembered grant opens. Upstream review explicitly raised
target replacement, revalidation and revocation.

Implement explicit per-open approval first. Keep remembered grants disabled
until the target identity and revalidation policy is agreed and tested.
If introduced later, ensure grants are bounded, revocable, validated on
settings restore, and invalidated on relevant source/target changes. Never
treat a JS proxy as evidence of consent.

## Integration points to inspect and change together

- `crates/js/src/globals.rs`: keep upstream argument validation, return
  restricted deferred proxy instead of `null`.
- `crates/js/src/model.rs` and `crates/js/src/host.rs`: request identity
  and optional page destination; preserve bounded request queues.
- `crates/js/src/session.rs`: confirm per-job request lifetime and that
  deferred handles cannot mutate requests from subsequent jobs.
- `crates/engine/src/js.rs`: carry the request unchanged through the engine.
- `crates/ui-egui/src/app/`: prompt, open/reuse tab, navigate on success,
  and discard canceled/closed-source requests.
- `crates/js/src/tests.rs`, engine and UI tests: add focused regressions.

## Acceptance tests

- Two independent `openDoc` handles with different pages and targets.
- Denied consent opens nothing and never navigates.
- Reused tab and in-flight tab navigate to the requested destination.
- Invalid path, forged `oDoc`, non-local `cFS`, invalid page, and request
  queue overflow cannot bypass restrictions.
- Closed source, canceled open, failed target, and stale script handle.
- Disabled JavaScript does not run link actions.
- Session reuse does not leak handles or grants across PDFs.
- `cargo fmt --all -- --check`, targeted Rust tests, clippy, and UI manual QA.

## Upstream context

- https://github.com/storytold/pdfcraft/pull/2081
- https://github.com/storytold/pdfcraft/pull/7144
- https://github.com/storytold/pdfcraft/pull/2081#issuecomment-6777307029

No build or test success is claimed by this document.
