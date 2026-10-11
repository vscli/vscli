# Extension document save foundation

The internal `nativeDocumentSave` service connects one retained native document
ID/version to the existing asynchronous save controller. Admission captures the
arrival mirror and native path, text epoch, revision, saved baseline, EOL,
settings/profile and workspace ownership. It does not focus or promote an
inactive or hidden model. The eight-entry caller table retains completed
receipts until their exact service response enters the bounded transport FIFO.

The controller retains one actual persistence worker until its terminal result
and thread join. It publishes the original model's successful receipt before
synchronizing mirrors and answering true. Newer edits remain dirty. Retirement
before authorization rejects work; retirement after authorization preserves the
real commit and native publication. Backend ambiguity and committed-but-unpublished
outcomes reject explicitly rather than returning an invented false or true.

Untitled documents use the original native Save As prompt and document identity.
Path selection has a 300-second human-wait phase; acceptance starts the
30-second preauthorization phase before synchronous dispatch. Failure clears
only that exact admission, and a service-tagged intent/worker cannot run without
its matching caller interest. Existing manual/autosave/close/quit saves keep
ordinary native ownership and require no JavaScript runtime.

The public JavaScript `TextDocument.save()` hook is still unconnected at this
foundation milestone. Trusted callback caller lookup, shared-document service
admission, exact originating-command human-wait suspension and actual public
host/terminal qualification belong to the next slice. Virtual/custom filesystem
saves, broader encodings and all upstream save participants remain incomplete.
Service FIFO can delay later extension document operations.

Local qualification: fourteen real-worker native ownership/integrity tests and
all 812 native library tests pass, with formatting, strict all-target locked
Clippy and a native build. Terminal save automation, format-on-save and
code-actions-on-save journeys pass, covering Unicode/CRLF, Undo/Redo, disk
restart, autosave, external conflicts, dirty close/quit, original keybinds,
late actual callbacks and terminal restoration. The ownership fixture bypasses
optional-host registration only; these tests do not establish public save API,
actual caller/session identity or didSave-before-promise compatibility.
A separate real-host transport regression fills the real 64-slot output queue,
retains the canceled Save As receipt across failed response admissions, then
observes exactly one false reply after release and a real ping. The same test
fails against the original removal-before-send implementation at the receipt
retention oracle. It also preserves the hidden Unicode/CRLF model, Undo/Redo,
active editor and disk bytes. This exercises the internal protocol; public
TextDocument.save and human command-wait behavior remain separate qualification.
No local VS Code instance or CI investigation was used.
