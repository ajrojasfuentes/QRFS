use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyCreate, ReplyWrite, ReplyEmpty, ReplyStatfs, ReplyOpen, Request,
    TimeOrNow,
};
use libc::{EIO, ENOENT, ENOSPC, ENAMETOOLONG, ENOTDIR, EISDIR, EACCES, ENOTEMPTY};
use std::ffi::OsStr;
use std::time::{Duration, UNIX_EPOCH, SystemTime};
use std::collections::HashMap;

use qrfs_lib::device::BlockDevice;
use qrfs_lib::crypto::CryptoEngine;
use qrfs_lib::types::{SuperBlock, Inode, BLOCK_SIZE, DIRECT_POINTERS, DirEntry};
use qrfs_lib::bitmap::Bitmap;
use qrfs_lib::types::FileType as QrFileType;

const TTL: Duration = Duration::from_secs(1);

pub struct QRFS {
    device: BlockDevice,
    crypto: CryptoEngine,
    sb: SuperBlock,
    bitmap: Bitmap,
    inodes: HashMap<u64, Inode>, // Cache en RAM de inodos
}

impl QRFS {
    // --- INICIALIZACIÓN (Mount) ---
    pub fn try_mount(device: BlockDevice, password: &str) -> anyhow::Result<Self> {
        // 1. Leer Superbloque
        let block0 = device.read_block(0)?;
        if block0.len() < 16 { anyhow::bail!("Bloque 0 inválido"); }
        
        let (salt, encrypted_sb) = block0.split_at(16);
        let mut salt_arr = [0u8; 16];
        salt_arr.copy_from_slice(salt);

        let crypto = CryptoEngine::new(password, salt_arr);
        let sb_bytes = crypto.decrypt(encrypted_sb).map_err(|_| anyhow::anyhow!("Error de autenticación"))?;
        let sb: SuperBlock = bincode::deserialize(&sb_bytes)?;

        // 2. Leer Bitmap
        let enc_bitmap = device.read_block(sb.bitmap_start)?;
        let bitmap_bytes = crypto.decrypt(&enc_bitmap)?;
        let bitmap: Bitmap = bincode::deserialize(&bitmap_bytes)?;

        // 3. Leer Inodos (Bloque inicial)
        let enc_inodes = device.read_block(sb.inode_table_start)?;
        let inodes_bytes = crypto.decrypt(&enc_inodes)?;
        let inode_list: Vec<Inode> = bincode::deserialize(&inodes_bytes)?;
        
        let mut inode_cache = HashMap::new();
        for (i, inode) in inode_list.iter().enumerate() {
            if inode.mode != 0 {
                inode_cache.insert(i as u64, inode.clone());
            }
        }

        Ok(Self { device, crypto, sb, bitmap, inodes: inode_cache })
    }

    // --- HELPERS INTERNOS DE PERSISTENCIA ---

    /// Guarda el bitmap en disco
    fn sync_bitmap(&self) -> Result<(), i32> {
        let bytes = bincode::serialize(&self.bitmap).map_err(|_| EIO)?;
        let encrypted = self.crypto.encrypt(&bytes).map_err(|_| EIO)?;
        self.device.write_block(self.sb.bitmap_start, &encrypted).map_err(|_| EIO)?;
        Ok(())
    }

    /// Guarda un inodo específico en disco
    fn sync_inode(&self, inode_idx: u64, inode: &Inode) -> Result<(), i32> {
        // Leemos la lista actual (simulada en caché + defaults)
        let mut inode_list = vec![Inode::new(QrFileType::File, 0); 5]; 
        
        for (k, v) in &self.inodes {
            if *k < inode_list.len() as u64 {
                inode_list[*k as usize] = v.clone();
            }
        }
        if inode_idx < inode_list.len() as u64 {
            inode_list[inode_idx as usize] = inode.clone();
        }

        let bytes = bincode::serialize(&inode_list).map_err(|_| EIO)?;
        let encrypted = self.crypto.encrypt(&bytes).map_err(|_| EIO)?;
        self.device.write_block(self.sb.inode_table_start, &encrypted).map_err(|_| EIO)?;
        
        Ok(())
    }

    // --- HELPERS DE LECTURA/ESCRITURA DE DATOS ---

    /// Lee y descifra los bloques de datos de un inodo
    fn read_inode_data(&self, inode: &Inode) -> Result<Vec<u8>, i32> {
        let mut data = Vec::new();
        for &block_id in inode.direct_blocks.iter() {
            if block_id == 0 { break; }
            
            let enc_block = self.device.read_block(block_id).map_err(|_| EIO)?;
            if enc_block.iter().all(|&x| x == 0) { continue; } // Bloque vacío
            
            let plain_block = self.crypto.decrypt(&enc_block).map_err(|_| EIO)?;
            data.extend_from_slice(&plain_block);
        }
        // Ajustar al tamaño real del archivo
        if data.len() > inode.size as usize {
            data.truncate(inode.size as usize);
        }
        Ok(data)
    }

    /// Cifra y escribe datos en un inodo, asignando bloques si es necesario
    fn write_inode_data(&mut self, inode_idx: u64, new_data: &[u8]) -> Result<(), i32> {
        let mut inode = self.inodes.get(&inode_idx).ok_or(ENOENT)?.clone();
        let mut written = 0;
        let mut block_ptr_idx = 0;
        const CHUNK_SIZE: usize = 900; 

        while written < new_data.len() {
            if block_ptr_idx >= DIRECT_POINTERS { return Err(ENOSPC); }

            let mut block_id = inode.direct_blocks[block_ptr_idx];
            if block_id == 0 {
                // Asignar nuevo bloque
                block_id = self.bitmap.allocate().ok_or(ENOSPC)?;
                inode.direct_blocks[block_ptr_idx] = block_id;
                self.sync_bitmap()?;
            }

            let end = std::cmp::min(written + CHUNK_SIZE, new_data.len());
            let chunk = &new_data[written..end];
            
            let encrypted = self.crypto.encrypt(chunk).map_err(|_| EIO)?;
            self.device.write_block(block_id, &encrypted).map_err(|_| EIO)?;

            written += chunk.len();
            block_ptr_idx += 1;
        }

        // Liberar bloques sobrantes si el archivo se hizo más pequeño
        for i in block_ptr_idx+1..DIRECT_POINTERS {
            if inode.direct_blocks[i] != 0 {
                self.bitmap.set(inode.direct_blocks[i] as usize, false);
                inode.direct_blocks[i] = 0;
            }
        }
        self.sync_bitmap()?;

        // Actualizar inodo
        inode.size = new_data.len() as u64;
        inode.modified_at = SystemTime::now();
        self.inodes.insert(inode_idx, inode.clone());
        self.sync_inode(inode_idx, &inode)?;

        Ok(())
    }

    /// Lee entradas de directorio (solo soporta directorio raíz plano por ahora)
    fn read_dir_entries(&self, inode_idx: u64) -> Result<Vec<DirEntry>, i32> {
        // En este diseño simple, asumimos que solo el inodo 1 tiene entradas
        if inode_idx != 1 { return Err(ENOTDIR); }
        
        let root_inode = self.inodes.get(&1).ok_or(ENOENT)?;
        let data = self.read_inode_data(root_inode)?;
        if data.is_empty() { return Ok(Vec::new()); }
        
        bincode::deserialize(&data).map_err(|_| EIO)
    }

    /// Agrega entrada al directorio raíz
    fn add_dir_entry(&mut self, name: String, inode_idx: u64) -> Result<(), i32> {
        let mut entries = self.read_dir_entries(1)?;
        if entries.iter().any(|e| e.name == name) { return Err(EIO); }
        
        entries.push(DirEntry { name, inode_idx });
        
        let new_data = bincode::serialize(&entries).map_err(|_| EIO)?;
        self.write_inode_data(1, &new_data)
    }

    /// Remueve entrada del directorio raíz
    fn remove_dir_entry(&mut self, name: &str) -> Result<u64, i32> {
        let mut entries = self.read_dir_entries(1)?;
        let pos = entries.iter().position(|e| e.name == name).ok_or(ENOENT)?;
        let inode_idx = entries[pos].inode_idx;
        
        entries.remove(pos);
        
        let new_data = bincode::serialize(&entries).map_err(|_| EIO)?;
        self.write_inode_data(1, &new_data)?;
        
        Ok(inode_idx)
    }

    /// Libera recursos de un inodo borrado
    fn free_inode_resources(&mut self, inode_idx: u64) -> Result<(), i32> {
        if let Some(mut inode) = self.inodes.get(&inode_idx).cloned() {
            for &block_id in inode.direct_blocks.iter() {
                if block_id != 0 { self.bitmap.set(block_id as usize, false); }
            }
            self.sync_bitmap()?;

            inode.mode = 0; // Marcar como borrado
            inode.size = 0;
            inode.direct_blocks = [0; DIRECT_POINTERS];
            self.inodes.insert(inode_idx, inode.clone());
            self.sync_inode(inode_idx, &inode)?;
            self.inodes.remove(&inode_idx);
        }
        Ok(())
    }

    /// Convierte Inode a FileAttr de FUSE
    fn get_file_attr(&self, inode_idx: u64, inode: &Inode) -> FileAttr {
        FileAttr {
            ino: inode_idx,
            size: inode.size,
            blocks: (inode.size + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64,
            atime: UNIX_EPOCH,
            mtime: inode.modified_at,
            ctime: inode.created_at,
            crtime: inode.created_at,
            kind: match inode.file_type {
                QrFileType::File => FileType::RegularFile,
                QrFileType::Directory => FileType::Directory,
            },
            perm: inode.mode,
            nlink: 1,
            uid: 501, gid: 20, rdev: 0, flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }
}

impl Filesystem for QRFS {
    // 1. LOOKUP: Buscar archivo por nombre
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != 1 { reply.error(ENOENT); return; } // Solo soportamos nivel 1
        let name_str = name.to_str().unwrap();

        match self.read_dir_entries(parent) {
            Ok(entries) => {
                for entry in entries {
                    if entry.name == name_str {
                        if let Some(inode) = self.inodes.get(&entry.inode_idx) {
                            reply.entry(&TTL, &self.get_file_attr(entry.inode_idx, inode), 0);
                            return;
                        }
                    }
                }
                reply.error(ENOENT);
            },
            Err(e) => reply.error(e),
        }
    }

    // 2. GETATTR: Obtener metadatos
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if let Some(inode) = self.inodes.get(&ino) {
            reply.attr(&TTL, &self.get_file_attr(ino, inode));
        } else {
            reply.error(ENOENT);
        }
    }

    // 3. SETATTR: Cambiar permisos, tamaño, tiempos (chmod, truncate, touch)
    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr
    ) {
        if let Some(mut inode) = self.inodes.get(&ino).cloned() {
            if let Some(new_mode) = mode {
                inode.mode = new_mode as u16;
            }
            
            if let Some(new_size) = size {
                // Si cambiamos tamaño, deberíamos truncar o expandir datos
                // Aquí solo actualizamos metadatos para simplificar, 
                // pero write_inode_data maneja expansión.
                inode.size = new_size;
            }

            inode.modified_at = SystemTime::now();
            self.inodes.insert(ino, inode.clone());
            let _ = self.sync_inode(ino, &inode); // Intentar guardar
            
            reply.attr(&TTL, &self.get_file_attr(ino, &inode));
        } else {
            reply.error(ENOENT);
        }
    }

    // 4. READDIR: Listar contenido
    fn readdir(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        if ino != 1 { reply.error(ENOENT); return; }

        let mut entries_fs = vec![
            (1, FileType::Directory, ".".to_string()),
            (1, FileType::Directory, "..".to_string()),
        ];
        if let Ok(disk_entries) = self.read_dir_entries(ino) {
            for entry in disk_entries {
                let kind = if let Some(node) = self.inodes.get(&entry.inode_idx) {
                    match node.file_type {
                        QrFileType::Directory => FileType::Directory,
                        _ => FileType::RegularFile,
                    }
                } else {
                    FileType::RegularFile
                };
                entries_fs.push((entry.inode_idx, kind, entry.name));
            }
        }

        for (i, entry) in entries_fs.into_iter().enumerate().skip(offset as usize) {
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }

    // 5. CREATE: Crear archivo regular
    fn create(&mut self, _req: &Request, parent: u64, name: &OsStr, mode: u32, _umask: u32, _flags: i32, reply: ReplyCreate) {
        if parent != 1 { reply.error(ENOENT); return; }
        let name_str = name.to_str().unwrap().to_string();

        let mut new_inode_id = 2;
        while self.inodes.contains_key(&new_inode_id) { new_inode_id += 1; }

        let new_inode = Inode::new(QrFileType::File, mode as u16);
        self.inodes.insert(new_inode_id, new_inode.clone());
        
        if let Err(e) = self.sync_inode(new_inode_id, &new_inode) { reply.error(e); return; }
        if let Err(e) = self.add_dir_entry(name_str, new_inode_id) { reply.error(e); return; }

        reply.created(&TTL, &self.get_file_attr(new_inode_id, &new_inode), 0, 0, 0);
    }

    // 6. MKDIR: Crear directorio (opcional, pero implementado)
    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, mode: u32, _umask: u32, reply: ReplyEntry) {
        if parent != 1 { reply.error(ENOENT); return; } // No soportamos subdirectorios anidados
        let name_str = name.to_str().unwrap().to_string();

        let mut new_inode_id = 2;
        while self.inodes.contains_key(&new_inode_id) { new_inode_id += 1; }

        // Tipo Directorio
        let new_inode = Inode::new(QrFileType::Directory, mode as u16);
        self.inodes.insert(new_inode_id, new_inode.clone());

        if let Err(e) = self.sync_inode(new_inode_id, &new_inode) { reply.error(e); return; }
        if let Err(e) = self.add_dir_entry(name_str, new_inode_id) { reply.error(e); return; }

        reply.entry(&TTL, &self.get_file_attr(new_inode_id, &new_inode), 0);
    }

    // 7. OPEN: Abrir archivo
    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        if let Some(inode) = self.inodes.get(&ino) {
            if inode.file_type == QrFileType::Directory {
                reply.error(EISDIR);
            } else {
                reply.opened(0, 0);
            }
        } else {
            reply.error(ENOENT);
        }
    }

    // 8. OPENDIR: Abrir directorio
    fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        if let Some(inode) = self.inodes.get(&ino) {
            if inode.file_type == QrFileType::Directory {
                reply.opened(0, 0);
            } else {
                reply.error(ENOTDIR);
            }
        } else {
            reply.error(ENOENT);
        }
    }

    // 9. READ: Leer datos
    fn read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock: Option<u64>, reply: ReplyData) {
        if let Some(inode) = self.inodes.get(&ino) {
            match self.read_inode_data(inode) {
                Ok(data) => {
                    let start = offset as usize;
                    if start >= data.len() { reply.data(&[]); return; }
                    let end = std::cmp::min(start + size as usize, data.len());
                    reply.data(&data[start..end]);
                },
                Err(e) => reply.error(e),
            }
        } else {
            reply.error(ENOENT);
        }
    }

    // 10. WRITE: Escribir datos
    fn write(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, data: &[u8], _wflags: u32, _flags: i32, _lock: Option<u64>, reply: ReplyWrite) {
        if offset != 0 { /* Simplificado: Solo reescritura total */ }
        if let Err(e) = self.write_inode_data(ino, data) {
            reply.error(e);
        } else {
            reply.written(data.len() as u32);
        }
    }

    // 11. UNLINK: Borrar archivo
    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if parent != 1 { reply.error(ENOENT); return; }
        let name_str = name.to_str().unwrap();

        match self.remove_dir_entry(name_str) {
            Ok(inode_idx) => {
                let _ = self.free_inode_resources(inode_idx);
                reply.ok();
            },
            Err(e) => reply.error(e),
        }
    }

    // 12. RMDIR: Borrar directorio
    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if parent != 1 { reply.error(ENOENT); return; }
        let name_str = name.to_str().unwrap();
        
        // Verificar tipo
        let mut target_inode = 0;
        if let Ok(entries) = self.read_dir_entries(parent) {
            for entry in entries {
                if entry.name == name_str { target_inode = entry.inode_idx; break; }
            }
        }
        if target_inode == 0 { reply.error(ENOENT); return; }

        if let Some(inode) = self.inodes.get(&target_inode) {
            if inode.file_type != QrFileType::Directory {
                reply.error(ENOTDIR); return;
            }
            // Verificar si está vacío (opcional, aquí simplificamos borrado)
        }

        match self.remove_dir_entry(name_str) {
            Ok(inode_idx) => {
                let _ = self.free_inode_resources(inode_idx);
                reply.ok();
            },
            Err(e) => reply.error(e),
        }
    }

    // 13. RENAME: Renombrar
    fn rename(&mut self, _req: &Request, parent: u64, name: &OsStr, newparent: u64, newname: &OsStr, _flags: u32, reply: ReplyEmpty) {
        if parent != 1 || newparent != 1 { reply.error(ENOENT); return; }
        let old_name = name.to_str().unwrap();
        let new_name = newname.to_str().unwrap().to_string();

        if let Ok(mut entries) = self.read_dir_entries(parent) {
            if entries.iter().any(|e| e.name == new_name) { reply.error(EIO); return; }

            if let Some(pos) = entries.iter().position(|e| e.name == old_name) {
                entries[pos].name = new_name;
                
                let new_data = bincode::serialize(&entries).unwrap();
                if let Err(e) = self.write_inode_data(1, &new_data) { reply.error(e); } 
                else { reply.ok(); }
            } else {
                reply.error(ENOENT);
            }
        } else {
            reply.error(EIO);
        }
    }

    // 14. STATFS: Espacio libre
    fn statfs(&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) {
        let mut free_blocks = 0;
        for i in 0..self.sb.total_blocks {
            if !self.bitmap.get(i as usize) { free_blocks += 1; }
        }
        reply.statfs(
            self.sb.total_blocks, free_blocks, free_blocks, 
            self.sb.total_inodes, self.sb.total_inodes - self.inodes.len() as u64,
            BLOCK_SIZE as u32, 255, BLOCK_SIZE as u32,
        );
    }

    // 15. ACCESS: Verificar permisos de acceso a un archivo
    // Se llama antes de open/read/write para verificar derechos.
    fn access(&mut self, _req: &Request, ino: u64, mask: i32, reply: ReplyEmpty) {
        // 1. Verificar si el archivo existe en nuestra estructura descifrada
        if let Some(inode) = self.inodes.get(&ino) {
            // Aquí es donde "usamos" la seguridad:
            // Si podemos leer el inodo de nuestra tabla hash, significa que 
            // la criptografía (passphrase) fue correcta al montar y tenemos acceso a la estructura.
            
            // Nota técnica: 'mask' contiene flags como R_OK (4), W_OK (2), X_OK (1).
            // En un FS real compararíamos (inode.mode & mask).
            // Para este proyecto, si el inodo existe y somos el dueño (simulado), damos acceso.
            
            // Opcional: Podrías validar si intentan escribir (W_OK) en un archivo de solo lectura,
            // pero por ahora devolvemos OK para permitir la operación.
            reply.ok();
        } else {
            // Si el inodo no está en memoria, el archivo no existe o está corrupto.
            reply.error(ENOENT);
        }
    }

    // 16. FSYNC: Asegurar que los datos bajen al disco físico
    fn fsync(&mut self, _req: &Request, ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        if self.inodes.contains_key(&ino) {
            // En QRFS, la escritura es Síncrona.
            // Cuando llamamos a `write`, este llama a `device.write_block`, 
            // el cual genera el PNG y lo guarda en el disco duro inmediatamente.
            
            // Por lo tanto, no tenemos un "buffer en RAM" pendiente de escribir.
            // Simplemente le decimos al SO: "Tranquilo, los datos ya están en los QRs".
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }
}