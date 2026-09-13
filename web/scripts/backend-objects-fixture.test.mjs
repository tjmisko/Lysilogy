import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

test('should execute the reported fixture when Cargo changes target directories or native target triples', async () => {
  const root = fileURLToPath(new URL('../..', import.meta.url));
  const ownedTarget = resolve(root, 'target');
  const executable = resolve(ownedTarget, 'debug/examples/objects_fixture');
  const originalSpawn = childProcess.spawnSync;
  const inheritedTarget = process.env.CARGO_TARGET_DIR;
  const inheritedTriple = process.env.CARGO_BUILD_TARGET;
  const nativeTriple = 'aarch64-unknown-linux-gnu';
  const cases = [
    { name: 'directory-environment', directoryEnvironment: true },
    { name: 'directory-configuration' },
    { name: 'native-triple-environment', directoryEnvironment: true, tripleEnvironment: nativeTriple },
    { name: 'native-triple-configuration', tripleConfiguration: nativeTriple },
  ];
  try {
    for (const setting of cases) {
      const configuredTarget = resolve(root, `different-${setting.name}-target`);
      if (setting.directoryEnvironment) process.env.CARGO_TARGET_DIR = configuredTarget;
      else delete process.env.CARGO_TARGET_DIR;
      if (setting.tripleEnvironment) process.env.CARGO_BUILD_TARGET = setting.tripleEnvironment;
      else delete process.env.CARGO_BUILD_TARGET;
      const current = { generation: 'current build', objects: [] };
      const artifacts = new Map([[executable, { generation: 'stale default build', objects: [] }]]);
      const calls = [];
      let reportedExecutable;
      childProcess.spawnSync = (command, args, options) => {
        calls.push({ command, args, options });
        if (command === 'cargo') {
          // Model Cargo's directory/target precedence. With the old helper this
          // leaves the subsequently executed default binary stale.
          const at = args.indexOf('--target-dir');
          const selected = at >= 0 ? args[at + 1] : options.env.CARGO_TARGET_DIR ?? configuredTarget;
          const triple = options.env.CARGO_BUILD_TARGET ?? setting.tripleConfiguration ?? '';
          reportedExecutable = resolve(selected, triple, 'debug/examples/objects_fixture');
          artifacts.set(reportedExecutable, current);
          const target = { name: 'objects_fixture', kind: ['example'], src_path: resolve(root, 'examples/objects_fixture.rs') };
          const messages = [
            { reason: 'compiler-artifact', target: { ...target, name: 'another_example' }, executable },
            { reason: 'compiler-artifact', target, executable: reportedExecutable },
            { reason: 'build-finished', success: true },
          ];
          return { status: 0, stdout: messages.map((message) => JSON.stringify(message)).join('\n'), stderr: '' };
        }
        return { status: 0, stdout: JSON.stringify(artifacts.get(command)), stderr: '' };
      };
      syncBuiltinESMExports();
      const { backendObjects } = await import(`./backend-objects-fixture.mjs?target-pairing-${setting.name}`);
      assert.deepEqual(backendObjects({ text: 'fixture input' }), current);
      assert.equal(calls.length, 2);
      assert.equal(calls[0].args[calls[0].args.indexOf('--target-dir') + 1], ownedTarget);
      assert.ok(calls[0].args.includes('--message-format=json'));
      assert.equal(calls[1].command, reportedExecutable);
      assert.equal(JSON.parse(calls[1].options.input).index.text, 'fixture input');
    }
  } finally {
    childProcess.spawnSync = originalSpawn;
    syncBuiltinESMExports();
    if (inheritedTarget === undefined) delete process.env.CARGO_TARGET_DIR;
    else process.env.CARGO_TARGET_DIR = inheritedTarget;
    if (inheritedTriple === undefined) delete process.env.CARGO_BUILD_TARGET;
    else process.env.CARGO_BUILD_TARGET = inheritedTriple;
  }
});
