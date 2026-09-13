// The mock HTTP transport serves output from the production Rust builder.
// No duplicate JavaScript bibliography parser or hand-written link answers.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../..', import.meta.url));
// This browser fixture owns the worktree target. Cargo may additionally select
// a target-triple subdirectory, so execute its reported compiler artifact.
const target = resolve(root, 'target');
let executable;
export const fixtureGeneration = '"objects-fixture-generation-1"';
export const fixturePaperId = '1111222233334444';

export function backendObjects(index, generation = fixtureGeneration, paperId = fixturePaperId) {
  if (executable === undefined) {
    const build = spawnSync('cargo', ['build', '--offline', '--example', 'objects_fixture', '--target-dir', target, '--message-format=json'], {
      cwd: root, encoding: 'utf8', timeout: 180_000, maxBuffer: 8 * 1024 * 1024,
      env: { ...process.env, CARGO_BUILD_JOBS: '1', CARGO_PROFILE_DEV_DEBUG: '0', CARGO_PROFILE_TEST_DEBUG: '0', CARGO_INCREMENTAL: '0' },
    });
    if (build.error || build.status !== 0) throw new Error(`Backend fixture build failed: ${build.error?.message ?? build.stderr}`);
    const candidates = new Set(build.stdout.split(/\r?\n/u).filter((line) => line.trim()).map((line) => JSON.parse(line))
      .filter((message) => message.reason === 'compiler-artifact' && message.target?.name === 'objects_fixture'
        && message.target.kind?.length === 1 && message.target.kind[0] === 'example'
        && typeof message.target.src_path === 'string' && resolve(root, message.target.src_path) === resolve(root, 'examples/objects_fixture.rs')
        && typeof message.executable === 'string' && message.executable.length > 0)
      .map((message) => resolve(root, message.executable)));
    if (candidates.size !== 1) throw new Error('Cargo must report exactly one production objects_fixture executable');
    [executable] = candidates;
  }
  const output = spawnSync(executable, [], {
    cwd: root, input: JSON.stringify({ index, generation, paper_id: paperId }), encoding: 'utf8',
    timeout: 10_000, maxBuffer: 8 * 1024 * 1024,
  });
  if (output.error || output.status !== 0) throw new Error(`Backend fixture failed: ${output.error?.message ?? output.stderr}`);
  return JSON.parse(output.stdout);
}
