import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

test('should execute the newly built fixture when Cargo settings point at a different target', async () => {
  const root = fileURLToPath(new URL('../..', import.meta.url));
  const ownedTarget = resolve(root, 'target');
  const executable = resolve(ownedTarget, 'debug/examples/objects_fixture');
  const originalSpawn = childProcess.spawnSync;
  const inheritedTarget = process.env.CARGO_TARGET_DIR;
  try {
    for (const setting of ['environment', 'configuration']) {
      const configuredTarget = resolve(root, `different-${setting}-target`);
      if (setting === 'environment') process.env.CARGO_TARGET_DIR = configuredTarget;
      else delete process.env.CARGO_TARGET_DIR;
      const current = { generation: 'current build', objects: [] };
      const artifacts = new Map([[executable, { generation: 'stale default build', objects: [] }]]);
      const calls = [];
      childProcess.spawnSync = (command, args, options) => {
        calls.push({ command, args, options });
        if (command === 'cargo') {
          // Model Cargo's CLI > environment > configuration target precedence.
          // With the old helper, this updates a different target and leaves the
          // subsequently executed default binary stale.
          const at = args.indexOf('--target-dir');
          const selected = at >= 0 ? args[at + 1] : options.env.CARGO_TARGET_DIR ?? configuredTarget;
          artifacts.set(resolve(selected, 'debug/examples/objects_fixture'), current);
          return { status: 0, stdout: '', stderr: '' };
        }
        return { status: 0, stdout: JSON.stringify(artifacts.get(command)), stderr: '' };
      };
      syncBuiltinESMExports();
      const { backendObjects } = await import(`./backend-objects-fixture.mjs?target-pairing-${setting}`);
      assert.deepEqual(backendObjects({ text: 'fixture input' }), current);
      assert.equal(calls.length, 2);
      assert.equal(calls[0].args[calls[0].args.indexOf('--target-dir') + 1], ownedTarget);
      assert.equal(calls[1].command, executable);
      assert.equal(JSON.parse(calls[1].options.input).index.text, 'fixture input');
    }
  } finally {
    childProcess.spawnSync = originalSpawn;
    syncBuiltinESMExports();
    if (inheritedTarget === undefined) delete process.env.CARGO_TARGET_DIR;
    else process.env.CARGO_TARGET_DIR = inheritedTarget;
  }
});
