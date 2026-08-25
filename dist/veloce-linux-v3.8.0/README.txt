================================================================================
 VeloceNetwork v3.8.0 (Windows Release)
 Enterprise Zero-Trust Mesh, OIDC SSO, and Distributed Systems Runtime
================================================================================

Quick Start:
  1. Start VeloceCore:
       .\veloce-core.exe run

  2. Open Web Status Portal (in your browser):
       .\veloce-run.exe portal   (or visit http://localhost:9090)

  3. Authenticate with Corporate Identity (SSO / ZTNA):
       .\veloce-run.exe auth login
       .\veloce-run.exe auth status

  4. Connect Machines & Mobile Devices:
       .\veloce-run.exe mesh identity
       .\veloce-run.exe mesh join <VM3-JOIN-CODE>

  5. Launch & Tunnel Services:
       .\veloce-run.exe --name web --hostname web.vln --port 8080 -- node app.js

Configuration:
  Policy & Role Bindings: veloce-policy.toml
