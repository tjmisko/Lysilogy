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

Tests run only the repository's synthetic C fixture, without TeX, papers, network
connections or model calls. They verify actual filesystem/process/syscall behavior
as well as policy generation. The required namespace tests fail instead of being
silently skipped. G5 explicitly registers bwrap and unshare in its tool PATH.

Selection and correspondence must be frozen before source execution. The
original feasibility proposal's exploratory criterion is retained: at least
8 of 10 builds and at least 5 of 10 **complete documents** reproducing every
original page at both 96 and 192 dpi. Counting five matching individual pages
would be weaker and is insufficient. Build failures stay in the selected ten.
Exact decoded whole-page pixels and dimensions are required; filenames, page
numbers, equation numbers and approximate image similarity cannot admit truth.
Any positive result still requires a separately reviewed source-layout codec,
complete per-kind inventory and original-PDF/source evidence.

Current implementation checkpoint: policy and synthetic process confinement
only. Runtime pinning, archive materialization, production engine commands,
selection, raster correspondence and the ten-paper experiment are not yet
complete or cleared for deposited-source execution.
