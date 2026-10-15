import {access,mkdir,readFile,writeFile,copyFile} from 'node:fs/promises';
import {join,dirname,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {labRoot} from '../src/registry.ts';
import {hashFile} from '../src/environment.ts';
import {sqliteLibrary} from '../src/registry.ts';
const exec=promisify(execFile);
// Qualified on macOS arm64 and on Linux (arm64, x86_64); the Go archive is pinned per platform.
const goArchives:Record<string,string>={'darwin-arm64':'ee215d57e0ec269c60cc9ceca68e6bda321ba9ee5afe24f4b0988703c2d87d12','linux-arm64':'3450b45a3f9ee8568792736a5c5e70a1f2e9b36c35a8f74958c03e51d7d92bec','linux-amd64':'63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445'};
const goPlatform=`${process.platform}-${process.arch==='x64'?'amd64':process.arch}`;if(!goArchives[goPlatform])throw new Error(`shared SQLite bootstrap not qualified for ${goPlatform}`);
const darwin=process.platform==='darwin';
const native=join(labRoot,'.native'),cache=join(labRoot,'.cache/sqlite-3.53.4'),toolchains=join(labRoot,'.toolchains');
await mkdir(native,{recursive:true});await mkdir(cache,{recursive:true});await mkdir(toolchains,{recursive:true});
async function download(url:string,path:string,sha:string){try{await access(path)}catch{const r=await fetch(url);if(!r.ok)throw new Error(`download ${r.status}: ${url}`);const bytes=Buffer.from(await r.arrayBuffer());if(createHash('sha256').update(bytes).digest('hex')!==sha)throw new Error('download checksum mismatch');await writeFile(path,bytes,{flag:'wx'})}if(await hashFile(path)!==sha)throw new Error(`cache checksum mismatch: ${path}`)}
const sqliteArchive=join(cache,'amalgamation.zip');const sqliteSha='1e71ddf93849c6a6ecf58b827c0692073d2dd7ee40196158068f7b29f422e87d';
await download('https://www.sqlite.org/2026/sqlite-amalgamation-3530400.zip',sqliteArchive,sqliteSha);
const sources=join(cache,'sqlite-amalgamation-3530400');await mkdir(sources,{recursive:true});
for(const file of ['sqlite3.c','sqlite3.h']){const data=(await exec('unzip',['-p',sqliteArchive,`sqlite-amalgamation-3530400/${file}`],{encoding:'buffer',maxBuffer:16*1024*1024})).stdout;const destination=join(sources,file);try{await access(destination);if(await hashFile(destination)!==createHash('sha256').update(data).digest('hex'))throw new Error(`extracted SQLite source mismatch: ${file}`)}catch(error){if((error as NodeJS.ErrnoException).code!=='ENOENT')throw error;await writeFile(destination,data,{flag:'wx'})}}
const goArchive=join(toolchains,`go1.27.1.${goPlatform}.tar.gz`);await download(`https://go.dev/dl/go1.27.1.${goPlatform}.tar.gz`,goArchive,goArchives[goPlatform]!);
async function run(command:string,args:string[],cwd=labRoot,env=process.env){await new Promise<void>((resolve,reject)=>{const child=spawn(command,args,{cwd,env,stdio:'inherit'});child.on('error',reject);child.on('exit',code=>code===0?resolve():reject(new Error(`${command} exited ${code}`)))})}
const go=join(toolchains,'go/bin/go');try{await access(go)}catch{await run('tar',['-xzf',goArchive],toolchains)}
const goVersion=(await exec(go,['version'])).stdout.trim();if(!goVersion.includes('go1.27.1'))throw new Error('unexpected local Go toolchain');
const sqliteSourceId='2026-07-24 19:02:57 bf7c7f30031888f4e796e429ab3978879485813aaca6f641c7b33e4e09459bcc';
const sourceSha3=createHash('sha3-256').update(await readFile(join(sources,'sqlite3.c'))).digest('hex');
if(sourceSha3!=='67f423e9ebbbdc473cbc4772c872ee6b89f31fde4ed0279a5c25d5f65c043a16')throw new Error('SQLite source differs from official release SHA3-256');
const common=join(labRoot,'adapters/sqlite-common');const library=join(native,sqliteLibrary);
const flags=['-O2','-fPIC','-fvisibility=hidden','-DSQLITE_THREADSAFE=1','-DSQLITE_ENABLE_COLUMN_METADATA','-DSQLITE_DEFAULT_WAL_AUTOCHECKPOINT=1000','-DSQLITE_DEFAULT_SYNCHRONOUS=2'];
await run('clang',[...flags,...(darwin?['-dynamiclib']:['-shared']),'-I',sources,'-I',common,join(sources,'sqlite3.c'),join(common,'lab_sqlite.c'),...(darwin?['-install_name',library]:['-Wl,-soname,'+sqliteLibrary,'-lpthread','-ldl','-lm']),'-o',library]);
const headers=resolve(dirname(process.execPath),'../include/node');await access(join(headers,'node_api.h'));
await run('clang',['-O2','-fPIC',...(darwin?['-bundle','-undefined','dynamic_lookup']:['-shared']),'-I',headers,'-I',common,join(common,'node_addon.c'),'-L',native,'-lgoodfetch_sqlite','-Wl,-rpath,'+native,'-o',join(native,'sqlite.node')]);
try{await access(join(labRoot,'adapters/storage-rust-sync/Cargo.lock'))}catch{await run('cargo',['generate-lockfile','--offline','--manifest-path','adapters/storage-rust-sync/Cargo.toml'])}
await run('cargo',['build','--release','--locked','--manifest-path','adapters/storage-rust-sync/Cargo.toml','--target-dir','adapters/rust/target/shared']);
await copyFile(join(labRoot,'adapters/rust/target/shared/release/goodfetch-storage-rust-sync'),join(native,'goodfetch-storage-rust-sync'));
await run(go,['build','-trimpath','-o',join(native,'goodfetch-storage-go-sync'),'.'],join(labRoot,'adapters/storage-go-sync'),{...process.env,GOTOOLCHAIN:'local',CGO_ENABLED:'1',CGO_LDFLAGS:`-L${native} -lgoodfetch_sqlite -Wl,-rpath,${native}`});
const hashes:Record<string,string>={};for(const file of [sqliteLibrary,'sqlite.node','goodfetch-storage-rust-sync','goodfetch-storage-go-sync'])hashes[file]=await hashFile(join(native,file));
const inputHashes:Record<string,string>={};for(const file of ['tools/build-shared-sqlite.ts','adapters/sqlite-common/lab_sqlite.c','adapters/sqlite-common/lab_sqlite.h','adapters/sqlite-common/node_addon.c','adapters/storage-rust-sync/src/main.rs','adapters/storage-rust-sync/build.rs','adapters/storage-rust-sync/Cargo.toml','adapters/storage-rust-sync/Cargo.lock','adapters/storage-go-sync/main.go','adapters/storage-go-sync/go.mod'])inputHashes[file]=await hashFile(join(labRoot,file));
await writeFile(join(native,'build.json'),JSON.stringify({sqlite:'3.53.4',sqliteSourceId,sqliteArchiveSha256:sqliteSha,sqliteSourceSha256:await hashFile(join(sources,'sqlite3.c')),sqliteSourceSha3:sourceSha3,flags,hashes,inputHashes,nodeHeadersSha256:await hashFile(join(headers,'node_api.h')),node:process.version,go:goVersion,rust:(await exec('rustc',['-Vv'])).stdout.trim(),clang:(await exec('clang',['--version'])).stdout.trim(),libraryPath:library},null,2)+'\n');
process.stdout.write(JSON.stringify({libraryPath:library,go:goVersion,hashes},null,2)+'\n');
