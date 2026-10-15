import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { access, readdir, mkdir, copyFile } from 'node:fs/promises';
import { cpus, totalmem, platform, arch, release } from 'node:os';
import { join, dirname } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { labRoot } from './registry.ts';
const exec = promisify(execFile);
export async function hashFile(path: string) { const hash = createHash('sha256'); for await (const chunk of createReadStream(path)) hash.update(chunk); return hash.digest('hex'); }
export async function copySourceSnapshot(directory:string,hashes:Record<string,string>){
  for(const [file,hash]of Object.entries(hashes)){const destination=join(directory,'sources',file);await mkdir(dirname(destination),{recursive:true});await copyFile(join(labRoot,file),destination);if(await hashFile(destination)!==hash)throw new Error(`source changed while freezing campaign: ${file}`)}
}
export async function sourceFingerprint() {
  const hashes: Record<string,string> = {};
  async function visit(directory:string) { for(const entry of await readdir(join(labRoot,directory),{withFileTypes:true})) {
    const path=join(directory,entry.name); if(entry.isDirectory() && !['node_modules','target'].includes(entry.name)) await visit(path);
    else if(entry.isFile() && /\.(ts|rs|c|h|go)$/.test(entry.name)) hashes[path]=await hashFile(join(labRoot,path));
  } }
  for(const directory of ['src','fixtures','adapters','tools','tests']) await visit(directory);
  for(const path of ['.node-version','package.json','pnpm-workspace.yaml','pnpm-lock.yaml','tsconfig.json','adapters/node/package.json','adapters/stagehand-v3/package.json','adapters/rust/Cargo.toml','adapters/rust/Cargo.lock','adapters/storage-rust/Cargo.toml','adapters/storage-rust/Cargo.lock','adapters/storage-rust-sync/Cargo.toml','adapters/storage-rust-sync/Cargo.lock','adapters/storage-go-sync/go.mod','adapters/workflow-rust/Cargo.toml','adapters/workflow-rust/Cargo.lock']) hashes[path]=await hashFile(join(labRoot,path));
  return hashes;
}
export function browserPath() {
  process.env.PLAYWRIGHT_BROWSERS_PATH = join(labRoot, '.browsers');
  const require = createRequire(join(labRoot, 'adapters/node/package.json'));
  return require('playwright').chromium.executablePath() as string;
}
export async function environment() {
  const executablePath = browserPath(); await access(executablePath);
  const browserVersion = (await exec(executablePath, ['--version'], { timeout: 5000 })).stdout.trim();
  const lockHashes: Record<string, string | null> = {};
  for (const file of ['pnpm-lock.yaml', 'adapters/rust/Cargo.lock', 'adapters/storage-rust/Cargo.lock']) { try { lockHashes[file] = await hashFile(join(labRoot, file)); } catch { lockHashes[file] = null; } }
  const binaryHashes: Record<string,string|null> = {};
  for(const file of ['adapters/rust/target/http/release/goodfetch-lab-spider','adapters/rust/target/chrome/release/goodfetch-lab-spider','adapters/rust/target/smart/release/goodfetch-lab-spider','adapters/rust/target/http/release/goodfetch-lab-storage']) { try { binaryHashes[file]=await hashFile(join(labRoot,file)); } catch { binaryHashes[file]=null; } }
  return { node: process.version, nodeExecutable: process.execPath, os: platform(), release: release(), architecture: arch(), cpu: cpus()[0]?.model, logicalCpuCount: cpus().length,
    totalMemoryBytes: totalmem(), browser: { executablePath, version: browserVersion, sha256: await hashFile(executablePath), requestedViewport: { width: 1280, height: 720 }, headless: true }, lockHashes, binaryHashes, sourceHashes: await sourceFingerprint() };
}
