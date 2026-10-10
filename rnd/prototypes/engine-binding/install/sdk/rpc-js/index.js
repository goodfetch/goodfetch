'use strict'
// PROTOTYPE thin SDK: resolve the `engined` binary from the matching optional platform package
// (esbuild style) and talk newline-delimited JSON-RPC to it over stdio.
const { spawn } = require('node:child_process')
const fs = require('node:fs')

function isMusl() {
  // Same order of checks as the napi-rs generated loader: ldd file, then process.report.
  try {
    if (fs.readFileSync('/usr/bin/ldd', 'utf8').includes('musl')) return true
  } catch {}
  try {
    const report = process.report && process.report.getReport && process.report.getReport()
    const header = typeof report === 'string' ? JSON.parse(report).header : report && report.header
    if (header && header.glibcVersionRuntime) return false
    if (header && Array.isArray(report.sharedObjects)) return report.sharedObjects.some((f) => f.includes('musl'))
  } catch {}
  return false
}

function candidates() {
  const { platform, arch } = process
  const exe = platform === 'win32' ? 'engined.exe' : 'engined'
  if (platform !== 'linux') return [[`@automan-proto/engined-${platform}-${arch}`, exe]]
  const libc = isMusl() ? 'musl' : 'gnu'
  // Per-libc packages first, then one static-musl package that serves both libcs (esbuild style).
  return [
    [`@automan-proto/engined-linux-${arch}-${libc}`, exe],
    [`@automan-proto/engined-linux-${arch}`, exe],
  ]
}

function enginedPath() {
  if (process.env.AUTOMAN_ENGINED_PATH) return process.env.AUTOMAN_ENGINED_PATH
  const tried = []
  for (const [pkg, exe] of candidates()) {
    try {
      return require.resolve(`${pkg}/bin/${exe}`)
    } catch (e) {
      tried.push(`${pkg} (${e.code || e.message})`)
    }
  }
  throw new Error(
    `automan: no engine binary for ${process.platform}-${process.arch}. Tried: ${tried.join(', ')}. ` +
      'The platform package is an optional dependency: reinstall without --omit=optional/--no-optional, ' +
      'and do not copy node_modules between operating systems. Or set AUTOMAN_ENGINED_PATH.',
  )
}

function ping() {
  return new Promise((resolve, reject) => {
    const child = spawn(enginedPath(), [], { stdio: ['pipe', 'pipe', 'inherit'], windowsHide: true })
    let buf = ''
    child.on('error', reject)
    child.stdout.on('data', (d) => {
      buf += d
      const nl = buf.indexOf('\n')
      if (nl >= 0) {
        child.stdin.end()
        resolve(JSON.parse(buf.slice(0, nl)).result)
      }
    })
    child.stdin.write('{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
  })
}

module.exports = { enginedPath, ping }
