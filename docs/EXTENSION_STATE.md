# Native extension state storage

The backend in `extension_state` stores bounded JSON key patches under the native configuration root, separately from imported profiles and extension execution consent. Global state is isolated by extension ID; workspace state additionally uses the canonical workspace path. Calls perform filesystem I/O and must run on workers.

Writes take a one-second bounded file lock, reread current state, apply one patch, and atomically replace the payload after syncing a temporary file. Concurrent instances preserve different-key updates. Malformed existing payloads are retained and rejected. Generated directories, payloads, and lock files reject symbolic links and nonregular files. This is ordinary filesystem integrity protection, not a hostile-process sandbox.

Each state file supports 1,024 keys and 256 KiB of serialized data, with 1 KiB keys, 10,000 JSON values, and 32 nesting levels. New values are limited to 64 KiB. An absent patch value deletes a key; JSON null remains a stored value. A failed directory sync after replacement can report an error after the replacement has committed.

Five native tests cover scope isolation, malformed/oversized retention, injected precommit failure, tree and byte limits, concurrent patch merging and lock retries, and Unix FIFO/symlink rejection. This foundation alone does not expose VS Code Memento APIs to extensions; that integration and its end-to-end qualification are separate work.
