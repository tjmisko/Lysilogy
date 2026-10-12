# Confined layout experiment

This explicitly invoked experiment is evaluation tooling for issue #110. It
publishes no K1 labels. The approximately 500-paper requirement in #97 remains
open. Deposited TeX must not execute until the complete runner, input selection
and synthetic evidence have an independent review.

The first implementation supports the native Linux aarch64 ABI only. A positive
seccomp allowlist rejects other architectures and unknown syscalls. Bubblewrap
runs inside a fresh empty network namespace created by unshare, without loopback
configuration. Mount, PID, IPC, UTS and cgroup namespaces and dropped capabilities
provide the filesystem/process boundary. Missing tools or capabilities fail.

Only exact trusted runtime files, a read-only source tree and finite pre-created
output inodes may be mounted. The output directory, root and procfs are read-only;
each admitted output file has a separate writable bind. Neither a writable tmpfs
nor a writable device directory is exposed. Per-file RLIMIT_FSIZE times the fixed
inode count bounds all output, including captured stdout and stderr. Renaming,
linking or creating another output file is unsupported. A package requiring an
unlisted auxiliary file may fail; the experiment never adds a writable directory
to repair it. SyncTeX may leave its bounded busy file because finalization needs
rename; that limitation must be measured separately from PDF build success.

The engine may use at most 768 MiB of address space and 45 CPU seconds per pass,
with two passes and 90 wall seconds total per paper. No process/thread creation,
socket, namespace operation, ioctl, io_uring or resource-limit change is allowed.
The initial execve is allowed, and any later exec remains restricted to the mounted
executables and the same hard limits. Only querying prlimit64 is permitted. The
host controller closes inherited descriptors, kills the launcher process group,
reaps it and retains an interruption/failure receipt. Bubblewrap's die-with-parent
behavior terminates its separate session/PID namespace when the launcher dies.

Tests use the repository's synthetic C fixture and generated TeX documents,
without deposited papers, network connections or model calls. They verify actual
filesystem/process/syscall behavior, fixed engine/format/shell controls, bounded
infinite compilation and changed mathematical raster content. Missing installed
tools or namespace capabilities fail instead of being silently skipped. G5
explicitly registers bwrap and unshare in its tool PATH.

Selection and correspondence must be frozen before source execution. The
original feasibility proposal's exploratory criterion is retained: at least
8 of 10 builds and at least 5 of 10 **complete documents** reproducing every
original page at both 96 and 192 dpi. Counting five matching individual pages
would be weaker and is insufficient. Build failures stay in the selected ten.
Exact decoded whole-page pixels and dimensions are required; filenames, page
numbers, equation numbers and approximate image similarity cannot admit truth.
Any positive result still requires a separately reviewed source-layout codec,
complete per-kind inventory and original-PDF/source evidence.

The trusted runtime inventory binds every installed TeX tree entry plus the
exact engine, format, configuration, font map, renderer and dynamic libraries.
Symlinks in the trusted tree are inventoried without reading their unmounted
outside targets. Source archives instead require only bounded regular members.
Materialization preserves the exact original bytes. An ambiguous source root
remains a failure; no source rewriting or engine fallback repairs it.

The original safe basename remains the TeX job name, preserving bibliography
lookup. One extra pre-created file, `pdflatex2.fls`, supports the fixed PID2
engine's recorder startup. Both final and busy recorder/SyncTeX artifacts remain
bounded. Missing font maps must not silently cause a bitmap-font substitution:
the installed system PDFTeX map is explicitly pinned. Raster bytes are captured
through bounded stdout because the renderer removes path outputs before writing.

Whole-document comparison refuses changed page counts before a renderer can
clamp an out-of-range page. Every required page is compared at both resolutions;
malformed or unavailable rasters remain failures with retained receipts. The
batch keeps every selected failure/unstarted paper and a 15-minute process
deadline. Source-layout semantic claims and K1 publication are always false.

`freeze.py` copies only exact committed source bytes into a fresh external-cache
snapshot, with the frozen selection/runtime and hashes of Python, bwrap and
unshare. `execute.py` requires isolated Python without site initialization,
verifies the exact launch and independent review, and compiles verified module
bytes directly. Adjacent bytecode cannot substitute for those sources. Every
sandbox invocation rechecks the reviewed confinement tool identities. Each build,
page-count query and render also receives the absolute batch deadline.
Sandbox setup consumes that deadline too. The controller checks immediately
before process creation and recomputes the wait budget afterward; expiration
during setup prevents a launch, and expiration during creation triggers cleanup.

The independently reviewed ten-paper experiment completed on 2026-09-13: one
paper built and zero complete documents matched. All selected failures remain
in the denominator. This is an exploratory no-go; no layout truth is admitted.
See [the measured report](../../../docs/experiment-reports/2026-09-13-kb-source-layout-probe.md).
Original inputs and the executed source snapshot remain immutable. A later
experiment needs a separately reviewed runtime and launch; synthetic results
establish only the tested capability.
