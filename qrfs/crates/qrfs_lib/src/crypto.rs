use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce // Or `Key`
};
use pbkdf2::pbkdf2;
use hmac::Hmac;
use sha2::Sha256;
use rand::{Rng, thread_rng};
use thiserror::Error;

// Constantes de seguridad
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32; // AES-256 necesita 32 bytes
const ITERATIONS: u32 = 100_000; // Estándar de seguridad decente

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Error de cifrado/descifrado")]
    EncryptionError,
    #[error("Datos corruptos o contraseña incorrecta")]
    DecryptionError,
}

/// Estructura que maneja la sesión criptográfica
pub struct CryptoEngine {
    cipher: Aes256Gcm,
    pub salt: [u8; SALT_LEN],
}

impl CryptoEngine {
    /// Crea un nuevo motor generando un Salt aleatorio (para mkfs)
    pub fn new_with_random_salt(password: &str) -> Self {
        let mut salt = [0u8; SALT_LEN];
        thread_rng().fill(&mut salt);
        
        Self::new(password, salt)
    }

    /// Reconstruye el motor con un Salt existente (para mount)
    pub fn new(password: &str, salt: [u8; SALT_LEN]) -> Self {
        let mut key = [0u8; KEY_LEN];
        
        // Derivar clave usando PBKDF2 (Password-Based Key Derivation Function 2)
        // Esto hace que sea lento para un atacante adivinar la contraseña
        pbkdf2::<Hmac<Sha256>>(
            password.as_bytes(),
            &salt,
            ITERATIONS,
            &mut key
        ).expect("HMAC can be initialized with any key length");

        let cipher = Aes256Gcm::new(&key.into());
        
        Self { cipher, salt }
    }

    /// Cifra datos. Retorna: [NONCE (12 bytes) | TEXTO CIFRADO | TAG (16 bytes)]
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        // Generar un Nonce (Number used once) aleatorio para cada bloque
        let mut nonce_bytes = [0u8; 12];
        thread_rng().fill(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Cifrar
        let ciphertext = self.cipher.encrypt(nonce, data)
            .map_err(|_| CryptoError::EncryptionError)?;

        // Empaquetar todo junto: Nonce + Ciphertext
        let mut result = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }

    /// Descifra datos. Espera formato: [NONCE | TEXTO CIFRADO]
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::DecryptionError);
        }

        // Extraer Nonce y Ciphertext
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Descifrar
        let plaintext = self.cipher.decrypt(nonce, ciphertext)
            .map_err(|_| CryptoError::DecryptionError)?;

        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let password = "passphrase_segura_del_proyecto";
        let data = b"Este es el superbloque secreto de QRFS";

        // 1. Simular creación (mkfs)
        let engine = CryptoEngine::new_with_random_salt(password);
        
        // 2. Cifrar
        let encrypted = engine.encrypt(data).expect("Fallo al cifrar");
        
        // Los datos cifrados deben ser diferentes a los originales y más largos (overhead)
        assert_ne!(data.to_vec(), encrypted);
        assert!(encrypted.len() > data.len());

        // 3. Simular montaje (mount) - Usamos el MISMO salt
        let engine_mount = CryptoEngine::new(password, engine.salt);
        
        // 4. Descifrar
        let decrypted = engine_mount.decrypt(&encrypted).expect("Fallo al descifrar");

        // 5. Verificar
        assert_eq!(data.to_vec(), decrypted);
    }

    #[test]
    fn test_wrong_password() {
        let password = "password123";
        let data = b"Secret data";
        
        let engine = CryptoEngine::new_with_random_salt(password);
        let encrypted = engine.encrypt(data).unwrap();

        // Intento de descifrar con otra clave
        let engine_hacker = CryptoEngine::new("password_incorrecto", engine.salt);
        let result = engine_hacker.decrypt(&encrypted);

        assert!(result.is_err());
    }
}