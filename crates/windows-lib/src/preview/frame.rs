use std::io::{self, Read};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    let width = u32::from_le_bytes(header[..4].try_into().expect("fixed header"));
    let height = u32::from_le_bytes(header[4..].try_into().expect("fixed header"));
    if width == 0 || height == 0 || width > 720 || height > 450 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid preview dimensions",
        ));
    }
    let mut pixels = vec![0; width as usize * height as usize * 4];
    reader.read_exact(&mut pixels)?;
    Ok(Frame {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_consecutive_frames_without_consuming_the_next_header() {
        let bytes = [1, 0, 0, 0, 1, 0, 0, 0, 10, 20, 30, 255].repeat(2);
        let mut input = bytes.as_slice();
        for _ in 0..2 {
            let frame = read_frame(&mut input).unwrap();
            assert_eq!((frame.width, frame.height), (1, 1));
            assert_eq!(frame.pixels, [10, 20, 30, 255]);
        }
        assert!(input.is_empty());
    }

    #[test]
    fn rejects_invalid_sizes_and_truncated_pixels() {
        for (w, h) in [(0u32, 10u32), (721, 450), (720, 451), (u32::MAX, u32::MAX)] {
            let bytes = [w.to_le_bytes(), h.to_le_bytes()].concat();
            assert_eq!(
                read_frame(&mut bytes.as_slice()).err().unwrap().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let bytes = [1u32.to_le_bytes(), 1u32.to_le_bytes()].concat();
        assert_eq!(
            read_frame(&mut bytes.as_slice()).err().unwrap().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
