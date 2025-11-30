use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Bitmap {
    pub bits: Vec<u8>,
    pub size: usize, // Cantidad total de bloques que rastreamos
}

impl Bitmap {
    /// Crea un mapa nuevo con todos los bloques marcados como LIBRES (0).
    pub fn new(total_blocks: usize) -> Self {
        // Necesitamos 1 byte por cada 8 bloques.
        // (total_blocks + 7) / 8 es una forma de hacer ceil div.
        let byte_len = (total_blocks + 7) / 8;
        Self {
            bits: vec![0; byte_len],
            size: total_blocks,
        }
    }

    /// Busca el primer bloque libre, lo marca como ocupado y devuelve su índice.
    pub fn allocate(&mut self) -> Option<u64> {
        for i in 0..self.size {
            if !self.get(i) {
                self.set(i, true);
                return Some(i as u64);
            }
        }
        None // Disco lleno
    }

    /// Marca un bloque específico (ej. los del sistema) como ocupado/libre.
    pub fn set(&mut self, index: usize, value: bool) {
        if index >= self.size { return; }
        
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        
        if value {
            self.bits[byte_idx] |= 1 << bit_idx; // Poner bit en 1
        } else {
            self.bits[byte_idx] &= !(1 << bit_idx); // Poner bit en 0
        }
    }

    /// Verifica si un bloque está ocupado.
    pub fn get(&self, index: usize) -> bool {
        if index >= self.size { return false; }
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.bits[byte_idx] & (1 << bit_idx)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitmap_allocation() {
        // Crear un mapa pequeño para 16 bloques
        let mut bitmap = Bitmap::new(16);

        // Verificar que todo empieza libre
        assert_eq!(bitmap.get(0), false);

        // Ocupar el primer bloque
        let first = bitmap.allocate().unwrap();
        assert_eq!(first, 0);
        assert_eq!(bitmap.get(0), true);

        // Ocupar manualmente el bloque 5
        bitmap.set(5, true);
        assert_eq!(bitmap.get(5), true);

        // Verificar que allocate() se salta los ocupados
        // Debería encontrar el 1 (el 0 está ocupado)
        let second = bitmap.allocate().unwrap();
        assert_eq!(second, 1);
    }
}