import { app, BrowserWindow, ipcMain } from 'electron'
import path from 'path'
import { spawn, ChildProcess } from 'child_process'

process.env.DIST = path.join(__dirname, '../dist')
process.env.PUBLIC = app.isPackaged ? process.env.DIST : path.join(process.env.DIST, '../public')

let win: BrowserWindow | null
let proxyProcess: ChildProcess | null = null

const VITE_DEV_SERVER_URL = process.env['VITE_DEV_SERVER_URL']

function createWindow() {
  win = new BrowserWindow({
    width: 900,
    height: 700,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
    },
  })

  if (VITE_DEV_SERVER_URL) {
    win.loadURL(VITE_DEV_SERVER_URL)
  } else {
    win.loadFile(path.join(process.env.DIST || '', 'index.html'))
  }
}

app.on('window-all-closed', () => {
  stopProxy()
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow()
  }
})

app.whenReady().then(createWindow)

// Proxy Management
function stopProxy() {
  if (proxyProcess) {
    proxyProcess.kill()
    proxyProcess = null
    win?.webContents.send('proxy-status', 'stopped')
    win?.webContents.send('proxy-log', '[System] Proxy stopped')
  }
}

ipcMain.on('stop-proxy', () => {
  stopProxy()
})

import fs from 'fs'

const CONFIG_PATH = path.join(app.getPath('userData'), 'proxy-config.json')

// ... (existing code) ...

// Config Persistence
ipcMain.handle('load-config', () => {
  try {
    if (fs.existsSync(CONFIG_PATH)) {
      return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf-8'))
    }
  } catch (e) {
    console.error('Failed to load config:', e)
  }
  return null
})

ipcMain.on('save-config', (_event, config) => {
  try {
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2))
  } catch (e) {
    console.error('Failed to save config:', e)
  }
})

ipcMain.on('start-proxy', (_event, config) => {
  // Save config on start
  try {
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2))
  } catch (e) { /* ignore */ }
  
  if (proxyProcess) return


  const { port, targetUrl, apiKey } = config
  
  // Resolve path to codex-proxy-anthropic.js
  let scriptPath: string
  if (app.isPackaged) {
    // In production, extraResources puts files in resources/
    scriptPath = path.join(process.resourcesPath, 'codex-proxy-anthropic.js')
  } else {
    // In dev: project_root/fronted/electron/main.ts -> project_root/codex-proxy-anthropic.js
    // main.ts is compiled to dist-electron/main.js, so we go up two levels
    scriptPath = path.resolve(__dirname, '../../codex-proxy-anthropic.js')
  }
  
  const env = { 
    ...process.env,
    PORT: port.toString(),
    CODEX_PROXY_TARGET: targetUrl,
    CODEX_API_KEY: apiKey
  }

  win?.webContents.send('proxy-log', `[System] Starting proxy on port ${port}...`)
  win?.webContents.send('proxy-log', `[System] Target: ${targetUrl}`)
  win?.webContents.send('proxy-log', `[System] Script: ${scriptPath}`)

  try {
    proxyProcess = spawn('node', [scriptPath], { env })

    proxyProcess.stdout?.on('data', (data) => {
      win?.webContents.send('proxy-log', data.toString().trim())
    })

    proxyProcess.stderr?.on('data', (data) => {
      win?.webContents.send('proxy-log', `[Error] ${data.toString().trim()}`)
    })

    proxyProcess.on('close', (code) => {
      win?.webContents.send('proxy-log', `[System] Process exited with code ${code}`)
      proxyProcess = null
      win?.webContents.send('proxy-status', 'stopped')
    })
    
    win?.webContents.send('proxy-status', 'running')

  } catch (error: any) {
    win?.webContents.send('proxy-log', `[Error] Failed to spawn: ${error.message}`)
    win?.webContents.send('proxy-status', 'stopped')
  }
})
