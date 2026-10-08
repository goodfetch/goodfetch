import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';
async function run(command: string, args: string[], env = process.env) {
  await new Promise<void>((resolve,reject) => { const child=spawn(command,args,{cwd:labRoot,env,stdio:'inherit'}); child.on('error',reject); child.on('exit',code=>code===0?resolve():reject(new Error(`${command} exited ${code}`))); });
}
await run('pnpm',['install','--frozen-lockfile']);
await run('pnpm',['--filter','@automan-lab/node-adapters','exec','playwright','install','chromium'],{...process.env,PLAYWRIGHT_BROWSERS_PATH:join(labRoot,'.browsers')});
for (const mode of ['http','chrome','smart']) {
  const args=['build','--release','--locked','--manifest-path','adapters/rust/Cargo.toml','--target-dir',`adapters/rust/target/${mode}`];
  if (mode!=='http') args.push('--features',mode);
  await run('cargo',args);
}
await run('cargo',['build','--release','--locked','--manifest-path','adapters/storage-rust/Cargo.toml','--target-dir','adapters/rust/target/http']);
await run('cargo',['build','--release','--locked','--manifest-path','adapters/cdp-rust/Cargo.toml']);
for (const stack of ['reqwest','wreq']) await run('cargo',['build','--release','--locked','--features',stack,'--manifest-path','adapters/http-rust/Cargo.toml','--target-dir',`adapters/http-rust/target/${stack}`]);
if(process.platform==='darwin'&&process.arch==='arm64')await run(process.execPath,['tools/build-shared-sqlite.ts']);
await run(process.execPath,['tools/doctor.ts']);
