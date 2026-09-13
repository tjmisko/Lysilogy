# Retained truth verifier

`k1-limited-v1/` contains the twelve exact application source modules used to
construct the first limited K1 release. Its manifest binds their original paths,
reviewed commit, source hashes and sizes, build configuration, and both immutable
output hashes. The total retained source is 257,691 bytes. These are application
sources; deposited paper sources remain in the external corpus.

`scripts/truth/latex/versioned.py` selects only this manifest and runs it in an
isolated Python process. A fixed module loader compiles the verified bytes
directly, so current parser changes, imported modules, `PYTHONPATH`, and cached
bytecode cannot silently change historical labels. The bundle's directory depth
preserves the original builder's repository-root calculation. Its imports do not
search this directory for other code.

Replay consumes the original explicit corpus, cache, and data roots; verifies
manual evidence and both frozen 1,000-paper ledgers; and compares reconstructed
object and bibliography payloads to the original hashes. It writes no truth,
index, registry, corpus, or bytecode files. New truth versions require an explicit
reviewed dispatch rather than falling back to this verifier or current modules.

Do not regenerate or update the retained modules when improving the active
parser. The versioned adapter's tests deliberately change current modules and
forge a valid cached-bytecode header to verify isolation.
