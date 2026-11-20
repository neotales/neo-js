use std::io::{self, Read};

use sha2::digest::DynDigest;

enum Hasher {
    Digest(Box<dyn DynDigest>),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    fn new(algorithm: &str) -> Result<Self, String> {
        let digest: Box<dyn DynDigest> = match algorithm {
            "sha224" => Box::<sha2::Sha224>::default(),
            "sha256" => Box::<sha2::Sha256>::default(),
            "sha384" => Box::<sha2::Sha384>::default(),
            "sha512" => Box::<sha2::Sha512>::default(),
            "sha3-224" => Box::<sha3::Sha3_224>::default(),
            "sha3-256" => Box::<sha3::Sha3_256>::default(),
            "sha3-384" => Box::<sha3::Sha3_384>::default(),
            "sha3-512" => Box::<sha3::Sha3_512>::default(),
            "blake2b-512" => Box::<blake2::Blake2b512>::default(),
            "blake2s-256" => Box::<blake2::Blake2s256>::default(),
            "blake3" => return Ok(Self::Blake3(Box::new(blake3::Hasher::new()))),
            _ => return Err(format!("unsupported hash algorithm: {algorithm}")),
        };
        Ok(Self::Digest(digest))
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Digest(hasher) => hasher.update(bytes),
            Self::Blake3(hasher) => {
                hasher.update(bytes);
            }
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Digest(hasher) => hex::encode(hasher.finalize()),
            Self::Blake3(hasher) => hasher.finalize().to_hex().to_string(),
        }
    }
}

fn hash_reader(algorithm: &str, mut reader: impl Read) -> Result<String, String> {
    let mut hasher = Hasher::new(algorithm)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => hasher.update(&buffer[..count]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(hasher.finish())
}

pub(crate) fn hash_bytes(algorithm: &str, data: &[u8]) -> Result<String, String> {
    hash_reader(algorithm, data)
}

pub(crate) fn hash_file(algorithm: &str, path: &str) -> Result<String, String> {
    // Reject an unknown algorithm before opening the file.
    Hasher::new(algorithm)?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    hash_reader(algorithm, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_vectors() {
        let vectors = [
            (
                "sha256",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "sha3-256",
                "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
            ),
            (
                "blake2s-256",
                "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982",
            ),
            (
                "blake3",
                "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
            ),
        ];
        for (algorithm, expected) in vectors {
            assert_eq!(hash_bytes(algorithm, b"abc").unwrap(), expected);
        }
    }

    #[test]
    fn hashes_large_files_and_rejects_unknown_algorithms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary");
        let data = vec![0xff; 200_000];
        std::fs::write(&path, &data).unwrap();
        let path = path.to_str().unwrap();
        assert_eq!(
            hash_file("sha512", path).unwrap(),
            hash_bytes("sha512", &data).unwrap()
        );
        assert!(hash_file("unknown", path).is_err());
    }
}
