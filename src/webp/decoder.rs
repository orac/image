use std::io;
use std::io::Read;
use std::default::Default;
use std::error::Error;
use byteorder::{ReadBytesExt, LittleEndian};

use image;
use image::ImageResult;
use image::ImageDecoder;

use color;

use nom::{le_u32, IResult};
use super::vp8::Frame;
use super::vp8::VP8Decoder;

// The "chunk size" item in a RIFF chunk specifies that "If Chunk Size is odd, a single padding byte -- that SHOULD be 0 -- is added." We need to parse the size, take (and return) that many bytes, and if the length was odd, drop one extra byte.
named!(chunk_size, do_parse!(
    len : le_u32 >>
    result : take!(len) >>
    cond!(len % 2 != 0, take!(1)) >>
    ( result )
));

named!(vp8_chunk, preceded!(
    tag!("VP8 "),
    chunk_size
));

named!(vp8l_chunk, preceded!(
    tag!("VP8L"),
    chunk_size
));

named!(vp8x_chunk, preceded!(
    tag!("VP8X"),
    chunk_size
));

named!(iccp_chunk, preceded!(
    tag!("ICCP"),
    chunk_size
));

named!(alph_chunk, preceded!(
    tag!("ALPH"),
    chunk_size
));

named!(exif_chunk, preceded!(
    tag!("EXIF"),
    chunk_size
));

named!(xmp_chunk, preceded!(
    tag!("XMP "),
    chunk_size
));

named!(extended<&[u8], ImageData>, chain!(
    vp8x_chunk ~
    opt!(iccp_chunk) ~
    // opt!(anim_chunk) ~ // don't support animations
    image_data : alt!(
        chain!(a : alph_chunk ~ rgb: vp8_chunk, || {ImageData::LossyWithAlpha(rgb, a)}) |
        map!(vp8_chunk, ImageData::Lossy) |
        map!(vp8l_chunk, ImageData::Lossless)
    ) ~
    // without the complete!, opt! will reach the end of the file and complain it can't decide whether the thing was there or not
    opt!(complete!(exif_chunk)) ~
    opt!(complete!(xmp_chunk)),
    || {image_data}
));

named!(webp_body<&[u8], ImageData>,
    alt!(
        map!(vp8_chunk, ImageData::Lossy) |
        map!(vp8l_chunk, ImageData::Lossless) |
        extended
    )
);

named!(webp_file<&[u8], ImageData>, preceded!(
    tag!("RIFF"),
    flat_map!(length_bytes!(le_u32), preceded!(
        tag!("WEBP"),
        webp_body
    ))
));


/// A Representation of a Webp Image format decoder.
pub struct WebpDecoder<R> {
    r: R,
    frame: Frame,
    have_frame: bool,
    decoded_rows: u32,
}

enum ImageData<'a> {
    Lossy(&'a[u8]),
    Lossless(&'a[u8]),
    LossyWithAlpha(&'a[u8], &'a[u8])
}

impl<R: Read> WebpDecoder<R> {
    /// Create a new WebpDecoder from the Reader ```r```.
    /// This function takes ownership of the Reader.
    pub fn new(r: R) -> WebpDecoder<R> {
        let f: Frame = Default::default();

        WebpDecoder {
            r: r,
            have_frame: false,
            frame: f,
            decoded_rows: 0
        }
    }

    fn read_vp8_frame(&mut self, framedata: &[u8]) -> ImageResult<()> {
        let m = io::Cursor::new(framedata);

        let mut v = VP8Decoder::new(m);
        let frame = try!(v.decode_frame());

        self.frame = frame.clone();

        Ok(())
    }

    fn read_metadata(&mut self) -> ImageResult<()> {
        if !self.have_frame {
            let mut everything = Vec::new();
            try!(self.r.read_to_end(&mut everything.as_mut()));
            match webp_file(everything.as_slice()) {
                IResult::Done(_, image) => {
                    match image {
                        ImageData::Lossy(vp8) | ImageData::LossyWithAlpha(vp8, _) => {
                            try!(self.read_vp8_frame(vp8));
                            self.have_frame = true;
                            Ok(())
                        },
                        ImageData::Lossless(_) =>
                            Err(image::ImageError::UnsupportedError(
                                String::from("Lossless WebP")
                            ))
                    }
                },
                IResult::Error(e) => Err(image::ImageError::FormatError(
                    format!("{}", e)
                )),
                IResult::Incomplete(needed) => {
                    Err(image::ImageError::NotEnoughData)
                }
            }
        } else {
            Ok(())
        }
    }
}

impl<R: Read> ImageDecoder for WebpDecoder<R> {
    fn dimensions(&mut self) -> ImageResult<(u32, u32)> {
        let _ = try!(self.read_metadata());

        Ok((self.frame.width as u32, self.frame.height as u32))
    }

    fn colortype(&mut self) -> ImageResult<color::ColorType> {
        Ok(color::ColorType::Gray(8))
    }

    fn row_len(&mut self) -> ImageResult<usize> {
        let _ = try!(self.read_metadata());

        Ok(self.frame.width as usize)
    }

    fn read_scanline(&mut self, buf: &mut [u8]) -> ImageResult<u32> {
        let _ = try!(self.read_metadata());

        if self.decoded_rows > self.frame.height as u32 {
            return Err(image::ImageError::ImageEnd)
        }

        let rlen  = buf.len();
        let slice = &self.frame.ybuf[
            self.decoded_rows as usize * rlen..
            self.decoded_rows as usize * rlen + rlen
        ];

        ::copy_memory(slice, buf);
        self.decoded_rows += 1;

        Ok(self.decoded_rows)
    }

    fn read_image(&mut self) -> ImageResult<image::DecodingResult> {
        let _ = try!(self.read_metadata());

        Ok(image::DecodingResult::U8(self.frame.ybuf.clone()))
    }
}
