use crate::errors::DecryptError::{self, *};
use crate::utils::{b64_decode, validate};

use aes::cipher::{
    block_padding::NoPadding, generic_array::typenum::consts::U16, generic_array::GenericArray,
    BlockDecryptMut, KeyIvInit,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use sha2::{Digest, Sha512};
use std::io::prelude::*;

// unused blocks are meant to verify password/file integrity
const _BLOCK1: [u8; 8] = [0xFE, 0xA7, 0xD2, 0x76, 0x3B, 0x4B, 0x9E, 0x79];
const _BLOCK2: [u8; 8] = [0xD7, 0xAA, 0x0F, 0x6D, 0x30, 0x61, 0x34, 0x4E];
const BLOCK3: [u8; 8] = [0x14, 0x6E, 0x0B, 0xE7, 0xAB, 0xAC, 0xD0, 0xD6];
const _BLOCK4: [u8; 8] = [0x5F, 0xB2, 0xAD, 0x01, 0x0C, 0xB9, 0xE1, 0xF6];
const _BLOCK5: [u8; 8] = [0xA0, 0x67, 0x7F, 0x02, 0xB2, 0x2C, 0x84, 0x33];

const SEGMENT_LENGTH: usize = 4096;

#[derive(Default, Debug)]
pub(crate) struct AgileEncryptionInfo {
    key_data_salt: Vec<u8>,
    key_data_hash_algorithm: String,
    key_data_block_size: u32,
    encrypted_hmac_key: Vec<u8>,
    encrypted_hmac_value: Vec<u8>,
    encrypted_verifier_hash_input: Vec<u8>,
    encrypted_verifier_hash_value: Vec<u8>,
    encrypted_key_value: Vec<u8>,
    spin_count: u32,
    password_salt: Vec<u8>,
    password_hash_algorithm: String,
    password_key_bits: u32,
}

impl AgileEncryptionInfo {
    pub fn new(mut encryption_info: impl Seek + Read) -> Result<Self, DecryptError> {
        encryption_info
            .seek(std::io::SeekFrom::Start(8))
            .map_err(|e| {
                InvalidStructure(format!("AgileEncryption: encryption_info.seek(8): {e}"))
            })?;
        let mut raw_xml = String::new();
        encryption_info.read_to_string(&mut raw_xml).map_err(|e| {
            InvalidStructure(format!(
                "AgileEncryption: encryption_info.read_to_string(): {e}"
            ))
        })?;

        // let raw_xml = String::from_utf8(encryption_info.stream[8..].to_vec())
        //     .map_err(|_| InvalidStructure)?;

        let mut reader = Reader::from_str(&raw_xml);
        reader.trim_text(true);

        let mut aei = Self::default();
        let mut set_key_data = false;
        let mut set_hmac_data = false;
        let mut set_password_node = false;

        loop {
            match reader.read_event().unwrap() {
                Event::Empty(e) => match e.name().as_ref() {
                    b"keyData" if !set_key_data => {
                        for attr in e.attributes() {
                            let attr = attr.map_err(|e| {
                                InvalidStructure(format!(
                                    "AgileEncryption: keyData: attributes(): {e}"
                                ))
                            })?;
                            match attr.key.as_ref() {
                                b"saltValue" => {
                                    aei.key_data_salt = b64_decode(&attr.value)?;
                                }
                                b"hashAlgorithm" => {
                                    aei.key_data_hash_algorithm = String::from_utf8(
                                        attr.value.into_owned(),
                                    )
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: keyData.hashAlgorithm: {e}"
                                        ))
                                    })?;
                                }
                                b"blockSize" => {
                                    aei.key_data_block_size = String::from_utf8(
                                        attr.value.into_owned(),
                                    )
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: keyData.blockSize: {e}"
                                        ))
                                    })?
                                    .parse()
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: keyData.blockSize: parse(): {e}"
                                        ))
                                    })?;
                                }
                                _ => (),
                            }
                        }
                        set_key_data = true;
                    }
                    b"dataIntegrity" if !set_hmac_data => {
                        for attr in e.attributes() {
                            let attr = attr.map_err(|e| {
                                InvalidStructure(format!(
                                    "AgileEncryption: dataIntegrity: attributes(): {e}"
                                ))
                            })?;
                            match attr.key.as_ref() {
                                b"encryptedHmacKey" => {
                                    aei.encrypted_hmac_key = b64_decode(&attr.value)?;
                                }
                                b"encryptedHmacValue" => {
                                    aei.encrypted_hmac_value = b64_decode(&attr.value)?;
                                }
                                _ => (),
                            }
                        }
                        set_hmac_data = true;
                    }
                    b"p:encryptedKey" if !set_password_node => {
                        for attr in e.attributes() {
                            let attr = attr.map_err(|e| {
                                InvalidStructure(format!(
                                    "AgileEncryption: p:encryptedKey: attributes(): {e}"
                                ))
                            })?;
                            match attr.key.as_ref() {
                                b"encryptedVerifierHashInput" => {
                                    aei.encrypted_verifier_hash_input = b64_decode(&attr.value)?;
                                }
                                b"encryptedVerifierHashValue" => {
                                    aei.encrypted_verifier_hash_value = b64_decode(&attr.value)?;
                                }
                                b"encryptedKeyValue" => {
                                    aei.encrypted_key_value = b64_decode(&attr.value)?;
                                }
                                b"spinCount" => {
                                    aei.spin_count = String::from_utf8(attr.value.into_owned())
                                        .map_err(|e| {
                                            InvalidStructure(format!(
                                                "AgileEncryption: p:encryptedKey.spinCount: {e}"
                                            ))
                                        })?
                                        .parse()
                                        .map_err(|e| {
                                            InvalidStructure(format!(
                                                "AgileEncryption: p:encryptedKey.spinCount: parse(): {e}"
                                            ))
                                        })?;
                                }
                                b"saltValue" => {
                                    aei.password_salt = b64_decode(&attr.value)?;
                                }
                                b"hashAlgorithm" => {
                                    aei.password_hash_algorithm = String::from_utf8(
                                        attr.value.into_owned(),
                                    )
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: p:encryptedKey.hashAlgorithm: {e}"
                                        ))
                                    })?;
                                }
                                b"keyBits" => {
                                    aei.password_key_bits = String::from_utf8(
                                        attr.value.into_owned(),
                                    )
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: p:encryptedKey.keyBits: {e}"
                                        ))
                                    })?
                                    .parse()
                                    .map_err(|e| {
                                        InvalidStructure(format!(
                                            "AgileEncryption: p:encryptedKey.keyBits: parse(): {e}"
                                        ))
                                    })?;
                                }
                                _ => (),
                            }
                        }
                        set_password_node = true;
                    }
                    _ => (),
                },
                Event::Eof => break,
                _ => (),
            }
        }

        validate!(
            set_key_data,
            InvalidStructure("AgileEncryption: keyData is missing".to_string())
        )?;
        validate!(
            set_hmac_data,
            InvalidStructure("AgileEncryption: dataIntegrity is missing".to_string())
        )?;
        validate!(
            set_password_node,
            InvalidStructure("AgileEncryption: p:encryptedKey is missing".to_string())
        )?;

        Ok(aei)
    }

    pub fn key_from_password(&self, password: &str) -> Result<Vec<u8>, DecryptError> {
        let digest = self.iterated_hash_from_password(password)?;
        let encryption_key = self.encryption_key(&digest, &BLOCK3)?;
        self.decrypt_aes_cbc(&encryption_key)
    }

    pub fn decrypt(
        &self,
        key: &[u8],
        mut encrypted_stream: impl Seek + Read,
    ) -> Result<Vec<u8>, DecryptError> {
        let mut bytes: [u8; 4] = [0; 4];
        encrypted_stream.read_exact(&mut bytes).map_err(|e| {
            InvalidStructure(format!(
                "AgileEncryption: decrypt: encrypted_steam.read_exact(4): {e}"
            ))
        })?;

        let total_size = u32::from_le_bytes(bytes) as usize;

        let mut block_start: usize = 8; // skip first 8 bytes
        let mut block_index: u32 = 0;
        let mut decrypted: Vec<u8> = vec![0; total_size];
        let key_data_salt: &[u8] = &self.key_data_salt;

        match self.key_data_hash_algorithm.as_str() {
            "SHA512" => {
                while block_start < (total_size - SEGMENT_LENGTH) {
                    let iv = Sha512::digest([key_data_salt, &block_index.to_le_bytes()].concat());
                    let iv = &iv[..16];

                    let cbc_cipher = cbc::Decryptor::<aes::Aes256>::new(key.into(), iv.into());

                    let mut in_buf: Vec<u8> = vec![];

                    encrypted_stream
                        .seek(std::io::SeekFrom::Start(block_start as u64))
                        .map_err(|e| {
                            InvalidStructure(format!(
                                "AgileEncryption: decrypt: SHA512: encrypted_stream(block_start): {e}"
                            ))
                        })?;
                    encrypted_stream
                        .by_ref()
                        .take(SEGMENT_LENGTH as u64)
                        .read_to_end(&mut in_buf)
                        .map_err(|e| {
                            InvalidStructure(format!(
                                "AgileEncryption: decrypt: SHA512: encrypted_stream: read segment: {e}"
                            ))
                        })?;

                    // decrypt from encrypted_stream directly to output Vec
                    cbc_cipher
                        .decrypt_padded_b2b_mut::<NoPadding>(
                            &in_buf,
                            &mut decrypted[(block_start - 8)..(block_start - 8 + SEGMENT_LENGTH)],
                        )
                        .map_err(|e| {
                            InvalidStructure(format!(
                                "AgileEncryption: decrypt: SHA512: cbc_cipher.decrypt: {e}"
                            ))
                        })?;

                    block_index += 1;
                    block_start += SEGMENT_LENGTH;
                }
                // parse last block w less than 4096 bytes
                let remaining = total_size - (block_start - 8);
                let iv = Sha512::digest([key_data_salt, &block_index.to_le_bytes()].concat());
                let iv = &iv[..16];

                let cbc_cipher = cbc::Decryptor::<aes::Aes256>::new(key.into(), iv.into());
                let irregular_block_len = remaining % 16;

                // remaining bytes in encrypted_stream should be a multiple of block size even if we only use some of the decrypted bytes
                let mut ciphertext: Vec<u8> = vec![];

                encrypted_stream
                    .seek(std::io::SeekFrom::Start(block_start as u64))
                    .map_err(|e| {
                        InvalidStructure(format!(
                            "AgileEncryption: decrypt: SHA512: encrypted_stream.seek(block_start): {e}"
                        ))
                    })?;
                encrypted_stream.read_to_end(&mut ciphertext).map_err(|e| {
                    InvalidStructure(format!(
                        "AgileEncryption: decrypt: SHA512: encrypted_stream: read remaining: {e}"
                    ))
                })?;

                validate!(
                    ciphertext.len() % 16 == 0,
                    InvalidStructure(
                        "AgileEncryption: decrypt: SHA512: remaining block size".to_string()
                    )
                )?;

                let mut plaintext: Vec<u8> = vec![0; ciphertext.len()];
                cbc_cipher
                    .decrypt_padded_b2b_mut::<NoPadding>(&ciphertext, &mut plaintext)
                    .map_err(|e| {
                        InvalidStructure(format!(
                            "AgileEncryption: decrypt: SHA512: cbc_cipher.decrypt(remaining): {e}"
                        ))
                    })?;
                let mut copy_span = plaintext.len() - 16 + irregular_block_len;
                if irregular_block_len == 0 {
                    copy_span += 16;
                }
                decrypted[(block_start - 8)..(block_start + copy_span - 8)]
                    .copy_from_slice(&plaintext[..copy_span]);
                Ok(decrypted)
            }
            "SHA1" | "SHA256" | "SHA384" => Err(Unimplemented(format!(
                "AgileEncryption: key_data_hash_algorithm: {}",
                self.password_hash_algorithm
            ))),
            _ => Err(InvalidStructure(
                "AgileEncryption: unrecognised key data hash algorithm".to_string(),
            )),
        }
    }

    // this function is ridiculously expensive as it usually runs 10000 SHA512's
    fn iterated_hash_from_password(&self, password: &str) -> Result<Vec<u8>, DecryptError> {
        let pass_utf16: Vec<u16> = password.encode_utf16().collect();
        let pass_utf16: &[u8] = unsafe { pass_utf16.align_to::<u8>().1 };
        let salted: Vec<u8> = [&self.password_salt, pass_utf16].concat();
        // TODO rewrite and pass ShaXXX:digest() as param?
        // could maybe abstract over T: Digest but the Sha512 type alias is weird
        match self.password_hash_algorithm.as_str() {
            "SHA512" => {
                let mut h = Sha512::digest(salted);
                for i in 0u32..self.spin_count {
                    h = Sha512::digest([&i.to_le_bytes(), h.as_slice()].concat());
                }

                Ok(h.as_slice().to_owned())
            }
            "SHA1" | "SHA256" | "SHA384" => Err(Unimplemented(format!(
                "AgileEncryption: password_hash_algorithm: {}",
                self.password_hash_algorithm
            ))),
            _ => Err(InvalidStructure(
                "AgileEncryption: unrecognised password hash algorithm".to_string(),
            )),
        }
    }

    fn encryption_key(&self, digest: &[u8], block: &[u8]) -> Result<Vec<u8>, DecryptError> {
        match self.password_hash_algorithm.as_str() {
            "SHA512" => {
                let h = Sha512::digest([digest, block].concat());
                Ok(h.as_slice()[..(self.password_key_bits as usize / 8)].to_owned())
            }
            "SHA1" | "SHA256" | "SHA384" => Err(Unimplemented(format!(
                "AgileEncryption: password_hash_algorithm: {}",
                self.password_hash_algorithm
            ))),
            _ => Err(InvalidStructure(
                "AgileEncryption: unrecognised password hash algorithm".to_string(),
            )),
        }
    }

    fn decrypt_aes_cbc(&self, key: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let mut cbc_cipher =
            cbc::Decryptor::<aes::Aes256>::new(key.into(), self.password_salt.as_slice().into());

        // two 16-byte cbc blocks
        // TODO how does the hash func affect # of blocks?
        let i1: GenericArray<u8, U16> =
            GenericArray::clone_from_slice(&self.encrypted_key_value.clone()[..16]);
        let i2: GenericArray<u8, U16> =
            GenericArray::clone_from_slice(&self.encrypted_key_value.clone()[16..]);
        let ciphertext_blocks = [i1, i2];

        let o1: GenericArray<u8, U16> = GenericArray::default();
        let o2: GenericArray<u8, U16> = GenericArray::default();
        let mut plaintext_blocks = [o1, o2];

        cbc_cipher
            .decrypt_blocks_b2b_mut(&ciphertext_blocks, &mut plaintext_blocks)
            .map_err(|_| Unknown)?;

        let plaintext = [
            plaintext_blocks[0].as_slice(),
            plaintext_blocks[1].as_slice(),
        ]
        .concat();

        Ok(plaintext)
    }
}