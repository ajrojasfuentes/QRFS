use std::fs;
use std::path::{Path, PathBuf};
use image::{Luma, imageops}; // <--- Agregamos imageops
use image::imageops::FilterType; // <--- Agregamos FilterType
use qrcode::{QrCode, Version, EcLevel};
use rqrr::PreparedImage;
use base64::{engine::general_purpose, Engine as _};
use thiserror::Error;

use crate::types::BLOCK_SIZE;

#[derive(Error, Debug)]
pub enum DeviceError {
    #[error("Error de IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error de codificación QR: {0}")]
    QrEncoding(#[from] qrcode::types::QrError),
    #[error("Error de imagen: {0}")]
    Image(#[from] image::ImageError),
    #[error("No se pudo decodificar el QR")]
    QrDecodingFailed,
    #[error("Error de decodificación Base64: {0}")]
    Base64Error(#[from] base64::DecodeError),
    #[error("El tamaño de los datos ({0}) excede el límite del bloque")]
    DataTooLarge(usize),
}

pub struct BlockDevice {
    root_path: PathBuf,
}

impl BlockDevice {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, DeviceError> {
        let root_path = path.as_ref().to_path_buf();
        if !root_path.exists() {
            fs::create_dir_all(&root_path)?;
        }
        Ok(Self { root_path })
    }

    fn get_path(&self, block_id: u64) -> PathBuf {
        self.root_path.join(format!("qr_{:05}.png", block_id))
    }

    /// ESCRIBIR: Bytes -> Base64 -> QR -> Imagen (177x177 aprox)
    pub fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), DeviceError> {
        if data.len() > BLOCK_SIZE {
            return Err(DeviceError::DataTooLarge(data.len()));
        }

        let b64_string = general_purpose::STANDARD.encode(data);
        let code = QrCode::with_version(b64_string, Version::Normal(40), EcLevel::L)?;

        // CAMBIO: Quitamos .max_dimensions(177, 177)
        // Permitimos que la librería genere el tamaño "natural" (que será 177 + borde).
        // .module_dimensions(1, 1) asegura que cada punto sea al menos 1 pixel.
        let image = code.render::<Luma<u8>>()
            .module_dimensions(1, 1) 
            .quiet_zone(true) // Asegura el borde blanco vital para la lectura
            .build();

        let path = self.get_path(block_id);
        image.save(path)?;

        Ok(())
    }

    /// LEER: Imagen -> Upscale (Zoom) -> Detectar QR -> Texto Base64 -> Bytes Originales
    pub fn read_block(&self, block_id: u64) -> Result<Vec<u8>, DeviceError> {
        let path = self.get_path(block_id);
        
        if !path.exists() {
            return Ok(vec![0u8; BLOCK_SIZE]);
        }

        // 1. Cargar imagen original (probablemente pequeña, 177x177)
        let img = image::open(path)?.to_luma8();
        
        // 2. TRUCO DE LA LUPA: Redimensionar en memoria
        // Escalamos a 400x400 usando 'Nearest' para mantener bordes nítidos.
        // Esto le da a 'rqrr' suficientes píxeles para detectar los patrones.
        let scaled_img = imageops::resize(&img, 400, 400, FilterType::Nearest);
        
        // 3. Convertir de vuelta a Luma8 para rqrr
        let dynamic_scaled = image::DynamicImage::ImageLuma8(scaled_img);
        let gray_scaled = dynamic_scaled.to_luma8();

        // 4. Preparar para lectura con la imagen grande
        let mut prepared_img = PreparedImage::prepare(gray_scaled);
        
        // 5. Detectar y Decodificar
        let grids = prepared_img.detect_grids();
        if let Some(grid) = grids.first() {
            // Extraer string Base64 del QR
            let (_meta, content_string) = grid.decode().map_err(|_| DeviceError::QrDecodingFailed)?;
            
            // Decodificar Base64 a Bytes originales
            let original_bytes = general_purpose::STANDARD.decode(content_string)?;
            return Ok(original_bytes);
        }

        Err(DeviceError::QrDecodingFailed)
    }

    pub fn count_blocks(&self) -> Result<u64, DeviceError> {
        let mut count = 0;
        // Leemos el directorio y contamos archivos .png
        if let Ok(entries) = fs::read_dir(&self.root_path) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("png") {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    /// Elimina físicamente los archivos QR que están fuera del nuevo rango.
    /// Útil para liberar espacio en el disco anfitrión al hacer shrink.
    pub fn trim(&self, start_block: u64, end_block: u64) -> Result<(), DeviceError> {
        for i in start_block..end_block {
            let path = self.get_path(i);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_write_and_read_base64_qr() {
        let test_dir = "test_qr";
        let _ = fs::remove_dir_all(test_dir); // Limpieza inicial

        let device = BlockDevice::new(test_dir).unwrap();
        
        // Creamos datos binarios "difíciles" (con ceros y caracteres de control)
        // para probar que el Base64 funciona bien.
        let original_data = vec![0x00, 0xFF, 0x10, 0x20, 0xCA, 0xFE, 0xBA, 0xBE];
        
        // Escribir
        device.write_block(0, &original_data).expect("Fallo al escribir");
        
        // Leer
        let read_data = device.read_block(0).expect("Fallo al leer");

        // Validar
        assert_eq!(original_data, read_data);
        
        // Limpieza final
        let _ = fs::remove_dir_all(test_dir);
    }
}
