/*!
VeloceVFS — Pure Userspace Inode-Based Virtual File System (v4.3).

Provides in-memory and containerized POSIX virtual filesystem semantics:
- Inodes, directories, regular files, symlinks, virtual devices, and dynamic `/proc` files.
- Copy-On-Write (COW) snapshot layers per workload instance.
- Direct integration with WebAssembly/WASI and VeloceCore workloads.
*/

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use veloce_ipc::message::{VfsEntryMsg, VfsEntryType, VfsListResultMsg, VfsReadResultMsg, VfsStatResultMsg};

/// Unique Inode Identifier.
pub type InodeId = u64;

/// Root Inode ID.
pub const ROOT_INODE_ID: InodeId = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InodeKind {
    Directory {
        /// Map of filename -> InodeId
        entries: BTreeMap<String, InodeId>,
    },
    File {
        content: Vec<u8>,
    },
    Symlink {
        target: String,
    },
    Device {
        device_name: String,
    },
    ProcFile {
        handler_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inode {
    pub id: InodeId,
    pub name: String,
    pub kind: InodeKind,
    pub size_bytes: u64,
    pub permissions: u32,
    pub created_at: u64,
    pub modified_at: u64,
}

impl Inode {
    pub fn entry_type(&self) -> VfsEntryType {
        match &self.kind {
            InodeKind::Directory { .. } => VfsEntryType::Directory,
            InodeKind::File { .. } => VfsEntryType::File,
            InodeKind::Symlink { .. } => VfsEntryType::Symlink,
            InodeKind::Device { .. } => VfsEntryType::Device,
            InodeKind::ProcFile { .. } => VfsEntryType::Proc,
        }
    }
}

/// Dynamic `/proc` generator hook.
pub type DynamicProcProvider = Arc<dyn Fn(&str) -> Result<Vec<u8>> + Send + Sync>;

/// Core Virtual File System Engine.
pub struct VfsEngine {
    pub inodes: RwLock<HashMap<InodeId, Inode>>,
    next_inode_id: AtomicU64,
    proc_provider: RwLock<Option<DynamicProcProvider>>,
}

impl VfsEngine {
    /// Initialize a new VeloceVFS instance with the standard OS directory hierarchy.
    pub fn new() -> Self {
        let engine = Self {
            inodes: RwLock::new(HashMap::new()),
            next_inode_id: AtomicU64::new(ROOT_INODE_ID + 1),
            proc_provider: RwLock::new(None),
        };

        engine.format_standard_layout().expect("init standard layout");
        engine
    }

    /// Set dynamic provider for live `/proc` files.
    pub fn set_proc_provider(&self, provider: DynamicProcProvider) {
        *self.proc_provider.write() = Some(provider);
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn alloc_inode_id(&self) -> InodeId {
        self.next_inode_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Reformat the filesystem into a clean standard POSIX micro-OS layout.
    pub fn format_standard_layout(&self) -> Result<()> {
        let mut inodes = self.inodes.write();
        inodes.clear();
        self.next_inode_id.store(ROOT_INODE_ID + 1, Ordering::SeqCst);

        let now = Self::now_secs();
        let root = Inode {
            id: ROOT_INODE_ID,
            name: "/".to_string(),
            kind: InodeKind::Directory { entries: BTreeMap::new() },
            size_bytes: 0,
            permissions: 0o755,
            created_at: now,
            modified_at: now,
        };
        inodes.insert(ROOT_INODE_ID, root);
        drop(inodes);

        // Create standard hierarchy
        self.mkdir_p("/bin")?;
        self.mkdir_p("/etc")?;
        self.mkdir_p("/dev")?;
        self.mkdir_p("/proc")?;
        self.mkdir_p("/var/log")?;
        self.mkdir_p("/mnt")?;
        self.mkdir_p("/tmp")?;
        self.mkdir_p("/vln/storage")?;

        // Standard system files
        self.write_file(
            "/etc/os-release",
            b"NAME=\"VeloceOS\"\nVERSION=\"4.3.0\"\nID=veloceos\nPRETTY_NAME=\"VeloceOS Userspace Micro-OS 4.3.0\"\nHOME_URL=\"https://github.com/LeTrollologist/veloce-workspace\"\n",
        )?;
        self.write_file(
            "/etc/hosts",
            b"127.0.0.1 localhost\n127.0.0.1 host.vln\n127.0.0.1 api.vln\n127.0.0.1 gateway.vln\n",
        )?;
        self.write_file(
            "/etc/resolv.conf",
            b"nameserver 127.0.0.1:5354\nsearch vln cluster.local\n",
        )?;

        // Virtual dynamic proc files
        self.register_proc_file("/proc/version", "version")?;
        self.register_proc_file("/proc/status", "status")?;
        self.register_proc_file("/proc/nodes", "nodes")?;
        self.register_proc_file("/proc/mesh", "mesh")?;
        self.register_proc_file("/proc/mounts", "mounts")?;

        // Virtual devices
        self.register_device("/dev/mesh", "mesh")?;
        self.register_device("/dev/null", "null")?;
        self.register_device("/dev/urandom", "urandom")?;

        Ok(())
    }

    /// Normalize a virtual path into components (e.g. "/a/b/../c" -> ["a", "c"]).
    pub fn normalize_path(path: &str) -> Vec<String> {
        let mut comps = Vec::new();
        for seg in path.replace('\\', "/").split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    comps.pop();
                }
                normal => {
                    comps.push(normal.to_string());
                }
            }
        }
        comps
    }

    /// Resolve an inode ID from a normalized path.
    pub fn lookup_path(&self, path: &str) -> Result<InodeId> {
        let comps = Self::normalize_path(path);
        if comps.is_empty() {
            return Ok(ROOT_INODE_ID);
        }

        let inodes = self.inodes.read();
        let mut current_id = ROOT_INODE_ID;

        for comp in &comps {
            let inode = inodes.get(&current_id).context("inode not found")?;
            match &inode.kind {
                InodeKind::Directory { entries } => {
                    if let Some(&next_id) = entries.get(comp) {
                        current_id = next_id;
                    } else {
                        bail!("no such file or directory: {}", path);
                    }
                }
                _ => bail!("not a directory in path traversal: {}", path),
            }
        }

        Ok(current_id)
    }

    /// Recursively create directories.
    pub fn mkdir_p(&self, path: &str) -> Result<InodeId> {
        let comps = Self::normalize_path(path);
        if comps.is_empty() {
            return Ok(ROOT_INODE_ID);
        }

        let mut current_id = ROOT_INODE_ID;
        let now = Self::now_secs();

        for comp in &comps {
            let mut inodes = self.inodes.write();
            let inode = inodes.get_mut(&current_id).context("parent inode not found")?;
            
            let next_id = match &mut inode.kind {
                InodeKind::Directory { entries } => {
                    if let Some(&existing) = entries.get(comp) {
                        existing
                    } else {
                        let new_id = self.alloc_inode_id();
                        entries.insert(comp.clone(), new_id);
                        new_id
                    }
                }
                _ => bail!("not a directory: {}", comp),
            };

            if !inodes.contains_key(&next_id) {
                let new_dir = Inode {
                    id: next_id,
                    name: comp.clone(),
                    kind: InodeKind::Directory { entries: BTreeMap::new() },
                    size_bytes: 0,
                    permissions: 0o755,
                    created_at: now,
                    modified_at: now,
                };
                inodes.insert(next_id, new_dir);
            }

            current_id = next_id;
        }

        Ok(current_id)
    }

    /// Write or overwrite a regular file at the given VFS path.
    pub fn write_file(&self, path: &str, content: &[u8]) -> Result<InodeId> {
        let comps = Self::normalize_path(path);
        if comps.is_empty() {
            bail!("cannot write to root path");
        }

        let (file_name, parent_comps) = comps.split_last().unwrap();
        let parent_path = if parent_comps.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", parent_comps.join("/"))
        };

        let parent_id = self.mkdir_p(&parent_path)?;
        let now = Self::now_secs();
        let content_len = content.len() as u64;

        let mut inodes = self.inodes.write();
        let parent_inode = inodes.get_mut(&parent_id).context("parent directory not found")?;
        
        let file_id = match &mut parent_inode.kind {
            InodeKind::Directory { entries } => {
                if let Some(&existing_id) = entries.get(file_name) {
                    existing_id
                } else {
                    let new_id = self.alloc_inode_id();
                    entries.insert(file_name.clone(), new_id);
                    new_id
                }
            }
            _ => bail!("parent is not a directory"),
        };

        let new_inode = Inode {
            id: file_id,
            name: file_name.clone(),
            kind: InodeKind::File { content: content.to_vec() },
            size_bytes: content_len,
            permissions: 0o644,
            created_at: now,
            modified_at: now,
        };

        inodes.insert(file_id, new_inode);
        Ok(file_id)
    }

    /// Read raw file contents from a VFS path (handles regular and dynamic `/proc` files).
    pub fn read_file(&self, path: &str) -> Result<VfsReadResultMsg> {
        let inode_id = self.lookup_path(path)?;
        let inodes = self.inodes.read();
        let inode = inodes.get(&inode_id).context("inode not found")?;

        match &inode.kind {
            InodeKind::File { content } => Ok(VfsReadResultMsg {
                path: path.to_string(),
                data: content.clone(),
                size_bytes: content.len() as u64,
            }),
            InodeKind::ProcFile { handler_name } => {
                if let Some(provider) = self.proc_provider.read().as_ref() {
                    let data = provider(handler_name).unwrap_or_else(|_| format!("dynamic proc error: {}", handler_name).into_bytes());
                    let len = data.len() as u64;
                    Ok(VfsReadResultMsg {
                        path: path.to_string(),
                        data,
                        size_bytes: len,
                    })
                } else {
                    let default_data = match handler_name.as_str() {
                        "version" => b"VeloceOS Userspace Micro-Kernel v4.3.0\n".to_vec(),
                        _ => format!("/proc/{} dynamic endpoint\n", handler_name).into_bytes(),
                    };
                    let len = default_data.len() as u64;
                    Ok(VfsReadResultMsg {
                        path: path.to_string(),
                        data: default_data,
                        size_bytes: len,
                    })
                }
            }
            InodeKind::Device { device_name } => {
                let data = format!("/dev/{} virtual device endpoint\n", device_name).into_bytes();
                let len = data.len() as u64;
                Ok(VfsReadResultMsg {
                    path: path.to_string(),
                    data,
                    size_bytes: len,
                })
            }
            _ => bail!("path is not a readable file: {}", path),
        }
    }

    /// Register a virtual device in `/dev`.
    pub fn register_device(&self, path: &str, device_name: &str) -> Result<InodeId> {
        let comps = Self::normalize_path(path);
        let (file_name, parent_comps) = comps.split_last().context("empty path")?;
        let parent_path = format!("/{}", parent_comps.join("/"));
        let parent_id = self.mkdir_p(&parent_path)?;
        let now = Self::now_secs();

        let mut inodes = self.inodes.write();
        let parent_inode = inodes.get_mut(&parent_id).context("parent not found")?;
        let dev_id = self.alloc_inode_id();

        if let InodeKind::Directory { entries } = &mut parent_inode.kind {
            entries.insert(file_name.clone(), dev_id);
        }

        let dev_inode = Inode {
            id: dev_id,
            name: file_name.clone(),
            kind: InodeKind::Device { device_name: device_name.to_string() },
            size_bytes: 0,
            permissions: 0o666,
            created_at: now,
            modified_at: now,
        };

        inodes.insert(dev_id, dev_inode);
        Ok(dev_id)
    }

    /// Register a dynamic `/proc` file.
    pub fn register_proc_file(&self, path: &str, handler_name: &str) -> Result<InodeId> {
        let comps = Self::normalize_path(path);
        let (file_name, parent_comps) = comps.split_last().context("empty path")?;
        let parent_path = format!("/{}", parent_comps.join("/"));
        let parent_id = self.mkdir_p(&parent_path)?;
        let now = Self::now_secs();

        let mut inodes = self.inodes.write();
        let parent_inode = inodes.get_mut(&parent_id).context("parent not found")?;
        let proc_id = self.alloc_inode_id();

        if let InodeKind::Directory { entries } = &mut parent_inode.kind {
            entries.insert(file_name.clone(), proc_id);
        }

        let proc_inode = Inode {
            id: proc_id,
            name: file_name.clone(),
            kind: InodeKind::ProcFile { handler_name: handler_name.to_string() },
            size_bytes: 0,
            permissions: 0o444,
            created_at: now,
            modified_at: now,
        };

        inodes.insert(proc_id, proc_inode);
        Ok(proc_id)
    }

    /// Stat metadata of an entry.
    pub fn stat(&self, path: &str) -> Result<VfsStatResultMsg> {
        let inode_id = self.lookup_path(path)?;
        let inodes = self.inodes.read();
        let inode = inodes.get(&inode_id).context("inode not found")?;

        Ok(VfsStatResultMsg {
            path: path.to_string(),
            entry_type: inode.entry_type(),
            size_bytes: inode.size_bytes,
            permissions: inode.permissions,
            modified_at_secs: inode.modified_at,
        })
    }

    /// List directory contents.
    pub fn list_dir(&self, path: &str) -> Result<VfsListResultMsg> {
        let inode_id = self.lookup_path(path)?;
        let inodes = self.inodes.read();
        let inode = inodes.get(&inode_id).context("inode not found")?;

        let mut result_entries = Vec::new();
        if let InodeKind::Directory { entries } = &inode.kind {
            for (name, &child_id) in entries {
                if let Some(child_inode) = inodes.get(&child_id) {
                    let child_path = if path == "/" {
                        format!("/{}", name)
                    } else {
                        format!("{}/{}", path.trim_end_matches('/'), name)
                    };

                    result_entries.push(VfsEntryMsg {
                        name: name.clone(),
                        path: child_path,
                        entry_type: child_inode.entry_type(),
                        size_bytes: child_inode.size_bytes,
                        modified_at_secs: child_inode.modified_at,
                        permissions: child_inode.permissions,
                    });
                }
            }
        } else {
            bail!("path is not a directory: {}", path);
        }

        Ok(VfsListResultMsg {
            path: path.to_string(),
            entries: result_entries,
        })
    }

    /// Import a host file into VFS.
    pub fn import_file(&self, host_path: &Path, vfs_path: &str) -> Result<u64> {
        let bytes = std::fs::read(host_path)
            .with_context(|| format!("failed to read host file: {}", host_path.display()))?;
        let len = bytes.len() as u64;
        self.write_file(vfs_path, &bytes)?;
        Ok(len)
    }

    /// Export a VFS file to the host filesystem.
    pub fn export_file(&self, vfs_path: &str, host_path: &Path) -> Result<u64> {
        let res = self.read_file(vfs_path)?;
        if let Some(parent) = host_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(host_path, &res.data)?;
        Ok(res.size_bytes)
    }

    /// Return total inode count and used VFS bytes.
    pub fn usage_metrics(&self) -> (usize, u64) {
        let inodes = self.inodes.read();
        let total_inodes = inodes.len();
        let mut total_bytes = 0u64;
        for inode in inodes.values() {
            if let InodeKind::File { content } = &inode.kind {
                total_bytes += content.len() as u64;
            }
        }
        (total_inodes, total_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vfs_file_and_directory_operations() {
        let vfs = VfsEngine::new();
        
        // Check root and default layout
        let root_stat = vfs.stat("/").unwrap();
        assert_eq!(root_stat.entry_type, VfsEntryType::Directory);

        let etc_list = vfs.list_dir("/etc").unwrap();
        assert!(etc_list.entries.iter().any(|e| e.name == "os-release"));
        assert!(etc_list.entries.iter().any(|e| e.name == "hosts"));

        // Read /etc/os-release
        let os_rel = vfs.read_file("/etc/os-release").unwrap();
        let content_str = String::from_utf8_lossy(&os_rel.data);
        assert!(content_str.contains("VeloceOS"));

        // Write custom file
        vfs.write_file("/vln/app/config.json", b"{\"port\":8080}").unwrap();
        let read_res = vfs.read_file("/vln/app/config.json").unwrap();
        assert_eq!(read_res.data, b"{\"port\":8080}");

        // Stat file
        let stat_res = vfs.stat("/vln/app/config.json").unwrap();
        assert_eq!(stat_res.entry_type, VfsEntryType::File);
        assert_eq!(stat_res.size_bytes, 13);
    }

    #[test]
    fn test_vfs_proc_and_dev_endpoints() {
        let vfs = VfsEngine::new();
        let dev_mesh = vfs.read_file("/dev/mesh").unwrap();
        assert!(String::from_utf8_lossy(&dev_mesh.data).contains("/dev/mesh"));

        let proc_ver = vfs.read_file("/proc/version").unwrap();
        assert!(String::from_utf8_lossy(&proc_ver.data).contains("VeloceOS"));
    }
}
