import { app, BrowserWindow, ipcMain, utilityProcess, UtilityProcess } from 'electron'
import path from 'path'
import { exec } from 'child_process'
import net from 'net'
import fs from 'fs'

process.env.DIST = path.join(__dirname, '../dist')
process.env.PUBLIC = app.isPackaged ? process.env.DIST : path.join(process.env.DIST, '../public')

let win: BrowserWindow | null
let proxyProcess: UtilityProcess | null = null

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

// Port Utilities
const checkPort = (port: number): Promise<boolean> => {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once('error', (err: any) => {
      if (err.code === 'EADDRINUSE') {
        resolve(true)
      } else {
        resolve(false)
      }
    })
    server.once('listening', () => {
      server.close()
      resolve(false)
    })
    server.listen(port)
  })
}

const killProcessOnPort = (port: number): Promise<void> => {
  return new Promise((resolve) => {
    const command = process.platform === 'win32'
      ? `for /f "tokens=5" %a in ('netstat -aon ^| findstr :${port}') do taskkill /f /pid %a`
      : `lsof -i :${port} -t | xargs kill -9`

    exec(command, () => resolve())
  })
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

ipcMain.on('start-proxy', async (_event, config) => {
  try {
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2))
  } catch (e) { /* ignore */ }
  
  const { port, targetUrl, apiKey, force } = config

  if (!force) {
    const isInUse = await checkPort(Number(port))
    if (isInUse) {
      win?.webContents.send('port-in-use', port)
      return
    }
  } else {
    win?.webContents.send('proxy-log', `[System] Stopping process on port ${port}...`)
    await killProcessOnPort(Number(port))
    await new Promise(r => setTimeout(r, 1000))
  }
  
  if (proxyProcess) return

  let scriptPath: string
  if (app.isPackaged) {
    scriptPath = path.join(process.resourcesPath, 'codex-proxy-anthropic.js')
  } else {
    scriptPath = path.resolve(__dirname, '../../codex-proxy-anthropic.js')
  }

  win?.webContents.send('proxy-log', `[System] Starting proxy on port ${port}...`)
  win?.webContents.send('proxy-log', `[System] Target: ${targetUrl}`)
  win?.webContents.send('proxy-log', `[System] Script: ${scriptPath}`)

  try {
    proxyProcess = utilityProcess.fork(scriptPath, [], {
      env: {
        ...process.env,
        PORT: port.toString(),
        CODEX_PROXY_TARGET: targetUrl,
        CODEX_API_KEY: apiKey
      }
    })

    proxyProcess.stdout?.on('data', (data) => {
      win?.webContents.send('proxy-log', data.toString().trim())
    })

    proxyProcess.stderr?.on('data', (data) => {
      win?.webContents.send('proxy-log', `[Error] ${data.toString().trim()}`)
    })

    proxyProcess.on('exit', (code) => {
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
