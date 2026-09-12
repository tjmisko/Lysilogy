# Vendored CodeMirror Vim releases

The two archives contain the real `@replit/codemirror-vim` 6.4.0 engine and its
`@replit/codemirror-vim-core` 0.1.0 dependency, built from the official
[v6.4.0 release](https://github.com/replit/codemirror-vim/tree/v6.4.0), commit
`4ca4d66bfaa12b3a5d8283b8c38532dec7328a68`. Each archive includes the upstream MIT
license, README, and `UPSTREAM.json`. `codemirror-vim-provenance.json` records the
actual local archive hashes; these are not represented as npm-published archives.

The npm registry was unavailable in the build environment. Official published
CodeMirror dependency archives were available in the local npm cache and retain
their standard registry URLs and SHA512 integrity in `package-lock.json`. These
two Vim archives came from the official Git repository because they were absent
from that cache. No editor commands or engine source were replaced or rewritten.

To reproduce, install the project's locked development dependencies, clone the
official repository, and check out the exact release commit:

```sh
git clone https://github.com/replit/codemirror-vim.git /tmp/codemirror-vim-upstream
git -C /tmp/codemirror-vim-upstream checkout 4ca4d66bfaa12b3a5d8283b8c38532dec7328a68
node vendor/build-codemirror-vim.mjs /tmp/codemirror-vim-upstream /tmp/rebuilt-vim
```

Run the Node command from `web/`. It verifies the commit and source modifications,
then uses the locked TypeScript 5.9.3 and esbuild 0.28.2. The core JavaScript is
copied unchanged. The adapter's own modules are bundled to ESM/CJS, leaving package
imports external and replacing the upstream `<DEV>` version marker. Declaration
files remain separate. Declaration emission uses `--noCheck` because the upstream
JavaScript's `continuePaste` JSDoc signature does not satisfy its own action-index
type under TypeScript 5.9; the application still checks its imports and adapter
types normally. Packaging removes development/lifecycle scripts and converts the
workspace core dependency to `^0.1.0`. Runtime sources remain unchanged.

Normal `npm ci` consumes the checked-in archives; it does not clone or build them.
The lockfile contains a complete, compatible and deduplicated set of CodeMirror
peers, so no persistent `legacy-peer-deps` configuration is needed.
