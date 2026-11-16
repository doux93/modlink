@echo off
rem ModuLink Studio dev launcher
rem When a local system proxy (e.g. Clash 127.0.0.1:7890) is on, WebView2 goes through
rem the proxy and the localhost dev page shows a blank screen. Force WebView2 to bypass it.
set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--no-proxy-server
cd /d %~dp0
npm run tauri dev