// Rebuild only the two vendored packages from the exact upstream release checkout.
import fs from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import esbuild from 'esbuild'

const here = path.dirname(fileURLToPath(import.meta.url))
const web = path.dirname(here)
const checkout = process.argv[2] && path.resolve(process.argv[2])
const output = path.resolve(process.argv[3] || here)
const cache = path.resolve(process.argv[4] || path.join(os.tmpdir(), 'lysilogy-vim-npm-cache'))
const commit = '4ca4d66bfaa12b3a5d8283b8c38532dec7328a68'
console.log('Checking the pinned upstream source')
if (!checkout) throw new Error('Usage: node vendor/build-codemirror-vim.mjs /path/to/upstream-checkout [output-directory]')
if (execFileSync('git', ['rev-parse', 'HEAD'], { cwd: checkout, encoding: 'utf8' }).trim() !== commit) {
  throw new Error(`Upstream checkout must be v6.4.0 at ${commit}`)
}
const sourceFiles = [
  'packages/codemirror-vim/src/index.ts', 'packages/codemirror-vim/src/cm_adapter.ts',
  'packages/codemirror-vim/src/block-cursor.ts', 'packages/codemirror-vim/package.json',
  'packages/codemirror-vim/LICENSE', 'packages/codemirror-vim/README.md',
  'packages/codemirror-vim/tsconfig.json', 'packages/codemirror-vim-core/package.json',
  'packages/codemirror-vim-core/vim.js', 'packages/codemirror-vim-core/types.ts',
  'packages/codemirror-vim-core/main.d.ts', 'packages/codemirror-vim-core/tsconfig.json',
  'packages/codemirror-vim-core/LICENSE', 'packages/codemirror-vim-core/README.md',
]
execFileSync('git', ['diff', '--exit-code', 'HEAD', '--', ...sourceFiles], { cwd: checkout, stdio: 'inherit' })
const typescript = JSON.parse(await fs.readFile(path.join(web, 'node_modules/typescript/package.json'), 'utf8')).version
if (typescript !== '5.9.3' || esbuild.version !== '0.28.2') {
  throw new Error('Reproducible archives require TypeScript 5.9.3 and esbuild 0.28.2 from the lockfile')
}
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'lysilogy-vim-build-'))
console.log(`Build directory: ${temporary}`)
await fs.mkdir(output, { recursive: true })
const json = (file, value) => fs.writeFile(file, `${JSON.stringify(value, null, 2)}\n`)
const tsc = path.join(web, 'node_modules/typescript/bin/tsc')
const built = []
for (const name of ['codemirror-vim-core', 'codemirror-vim']) {
  const upstream = path.join(checkout, 'packages', name)
  const stage = path.join(temporary, name)
  await fs.mkdir(path.join(stage, 'dist'), { recursive: true })
  const metadata = JSON.parse(await fs.readFile(path.join(upstream, 'package.json'), 'utf8'))
  delete metadata.scripts
  delete metadata.devDependencies
  const core = name.endsWith('-core')
  console.log(`Building ${name}`)
  const files = core ? ['vim.js', 'main.d.ts', 'LICENSE', 'README.md'] : ['LICENSE', 'README.md']
  for (const file of files) await fs.copyFile(path.join(upstream, file), path.join(stage, file))
  if (core) {
    execFileSync(process.execPath, [tsc, '-p', path.join(upstream, 'tsconfig.json'), '--outDir', path.join(stage, 'dist'), '--noCheck'], { stdio: 'inherit' })
  } else {
    metadata.dependencies['@replit/codemirror-vim-core'] = '^0.1.0'
    for (const format of ['esm', 'cjs']) {
      const result = await esbuild.build({
        absWorkingDir: upstream, entryPoints: ['src/index.ts'], bundle: true,
        packages: 'external', platform: 'browser', format, target: 'es2018',
        outfile: path.join(stage, 'dist', format === 'esm' ? 'index.js' : 'index.cjs'), write: false,
      })
      await fs.writeFile(result.outputFiles[0].path, result.outputFiles[0].text.replace('<DEV>', metadata.version))
    }
    const config = path.join(temporary, 'vim-types.json')
    await json(config, {
      extends: path.join(upstream, 'tsconfig.json'),
      compilerOptions: {
        types: [], outDir: path.join(stage, 'dist'), noCheck: true,
        paths: {
          '@codemirror/*': [path.join(web, 'node_modules/@codemirror/*')],
          '@replit/*': [path.join(web, 'node_modules/@replit/*')],
        },
      },
    })
    execFileSync(process.execPath, [tsc, '-p', config], { stdio: 'inherit' })
  }
  metadata.files = core ? ['vim.js', 'main.d.ts', 'dist', 'LICENSE', 'README.md', 'UPSTREAM.json'] : ['dist', 'LICENSE', 'README.md', 'UPSTREAM.json']
  await json(path.join(stage, 'package.json'), metadata)
  await json(path.join(stage, 'UPSTREAM.json'), {
    repository: 'https://github.com/replit/codemirror-vim', tag: 'v6.4.0', commit,
    path: `packages/${name}`,
    build: core
      ? 'Runtime vim.js unchanged. TypeScript 5.9.3 declaration-only emit with --noCheck; package lifecycle/development scripts removed.'
      : 'Runtime sources unchanged. esbuild 0.28.2 ES2018 ESM/CJS with package imports external; <DEV> replaced by 6.4.0, matching the upstream release hook. TypeScript 5.9.3 declaration-only --noCheck, unbundled declarations. Workspace dependency normalized to ^0.1.0; lifecycle/development scripts removed.',
  })
  console.log(`Packing ${name}`)
  const packed = JSON.parse(execFileSync('npm', ['pack', stage, '--offline', '--ignore-scripts', '--cache', cache, '--pack-destination', output, '--json'], { encoding: 'utf8' }))
  built.push({ package: packed[0].name, version: packed[0].version, file: packed[0].filename, integrity: packed[0].integrity, upstreamCommit: commit })
}
await json(path.join(output, 'codemirror-vim-provenance.json'), built)
console.log(JSON.stringify(built, null, 2))
