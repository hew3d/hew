//! Specs for reading a picture's size and building its placement recipe.

use super::*;

/// The first 24 bytes of a PNG `w` x `h` pixels (signature + IHDR start).
pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0]);
    b
}

/// A JPEG header: SOI, an optional Exif APP1 with `orientation`, a DQT-ish
/// filler segment, then a baseline SOF0 of `w` x `h`.
fn jpeg_header(w: u16, h: u16, orientation: Option<u16>, big_endian: bool) -> Vec<u8> {
    let mut b = vec![0xff, 0xd8];
    if let Some(o) = orientation {
        let mut tiff: Vec<u8> = Vec::new();
        let p16 = |v: u16| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let p32 = |v: u32| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        tiff.extend_from_slice(if big_endian { b"MM" } else { b"II" });
        tiff.extend_from_slice(&p16(42));
        tiff.extend_from_slice(&p32(8));
        tiff.extend_from_slice(&p16(2)); // two entries
        // An unrelated tag first (ImageWidth, LONG).
        tiff.extend_from_slice(&p16(0x0100));
        tiff.extend_from_slice(&p16(4));
        tiff.extend_from_slice(&p32(1));
        tiff.extend_from_slice(&p32(640));
        // Orientation, SHORT, one value, left-justified in the value field.
        tiff.extend_from_slice(&p16(0x0112));
        tiff.extend_from_slice(&p16(3));
        tiff.extend_from_slice(&p32(1));
        tiff.extend_from_slice(&p16(o));
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&p32(0));
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend_from_slice(&tiff);
        b.extend_from_slice(&[0xff, 0xe1]);
        b.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
        b.extend_from_slice(&seg);
    }
    // A filler segment (DQT, contents irrelevant), with fill bytes before
    // its marker.
    b.extend_from_slice(&[0xff, 0xff, 0xdb, 0x00, 0x04, 0x00, 0x00]);
    b.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    b.extend_from_slice(&[0xff, 0xd9]);
    b
}

#[test]
fn reads_a_pngs_size_from_its_header() {
    let info = read_info(&png_header(640, 480)).unwrap();
    assert_eq!(
        info,
        ImageInfo {
            format: ImageFormat::Png,
            width: 640,
            height: 480
        }
    );
}

#[test]
fn reads_a_jpegs_size_from_its_frame_header() {
    let info = read_info(&jpeg_header(1024, 768, None, false)).unwrap();
    assert_eq!(
        (info.format, info.width, info.height),
        (ImageFormat::Jpeg, 1024, 768)
    );
}

/// A phone photo taken upright is often stored on its side with an EXIF
/// orientation saying so; browsers show it upright, so its shown width is
/// the stored height.
#[test]
fn a_jpeg_stored_on_its_side_is_measured_as_shown() {
    for be in [false, true] {
        for o in 5..=8 {
            let info = read_info(&jpeg_header(4000, 3000, Some(o), be)).unwrap();
            assert_eq!((info.width, info.height), (3000, 4000), "orientation {o}");
        }
        for o in 1..=4 {
            let info = read_info(&jpeg_header(4000, 3000, Some(o), be)).unwrap();
            assert_eq!((info.width, info.height), (4000, 3000), "orientation {o}");
        }
    }
}

#[test]
fn refuses_bytes_that_are_not_a_picture() {
    for bytes in [
        b"GIF89a....".to_vec(),
        Vec::new(),
        png_header(10, 10)[..20].to_vec(),
        vec![0xff, 0xd8, 0xff, 0xd9],
        vec![0xff, 0xd8, 0xff, 0xc0, 0x00],
        png_header(0, 10),
    ] {
        assert!(
            matches!(read_info(&bytes), Err(ImageError::Unreadable(_))),
            "{bytes:?}"
        );
    }
}

#[test]
fn refuses_a_picture_over_64_megapixels() {
    assert_eq!(read_info(&png_header(8000, 8000)).unwrap().width, 8000);
    assert_eq!(
        read_info(&png_header(8001, 8000)),
        Err(ImageError::TooLarge {
            width: 8001,
            height: 8000
        })
    );
    assert_eq!(
        read_info(&png_header(8001, 8000)).unwrap_err().code(),
        "ImageTooLarge"
    );
}

#[test]
fn names_the_picture_after_its_file() {
    assert_eq!(picture_name("Kitchen wall.JPG"), "Kitchen wall");
    assert_eq!(picture_name("/tmp/plans/floor.plan.png"), "floor.plan");
    assert_eq!(picture_name("C:\\pics\\a.jpeg"), "a");
    assert_eq!(picture_name("notes.webp"), "notes.webp");
    assert_eq!(picture_name(".png"), DEFAULT_NAME);
    assert_eq!(picture_name("  "), DEFAULT_NAME);
}

#[test]
fn an_api_placement_defaults_to_one_meter_on_the_ground_at_the_origin() {
    assert_eq!(
        Placement::from_parts(None, None, None, None),
        Placement::ground(1.0)
    );
    let p = Placement::from_parts(
        Some([1.0, 2.0, 3.0]),
        None,
        Some([0.0, 0.0, 1.0]),
        Some(2.5),
    );
    assert_eq!(p.origin, Point3::new(1.0, 2.0, 3.0));
    assert_eq!(p.u, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(p.v, Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(p.width, 2.5);
}

#[test]
fn the_pose_is_a_rigid_frame_with_its_up_square_to_its_width() {
    let p = Placement {
        origin: Point3::new(1.0, 2.0, 3.0),
        u: Vec3::new(2.0, 0.0, 0.0),
        v: Vec3::new(1.0, 0.0, 5.0),
        width: 1.0,
    };
    let pose = p.pose().unwrap();
    let a = pose.to_affine();
    let col = |c: usize| Vec3::new(a[c], a[4 + c], a[8 + c]);
    let (x, y, z) = (col(0), col(1), col(2));
    let close = |v: Vec3, w: Vec3| (v - w).length() < 1e-12;
    assert!(close(x, Vec3::new(1.0, 0.0, 0.0)));
    assert!(close(y, Vec3::new(0.0, 0.0, 1.0)));
    assert!(close(z, Vec3::new(0.0, -1.0, 0.0)));
    assert!((pose.determinant() - 1.0).abs() < 1e-12);
    assert_eq!((a[3], a[7], a[11]), (1.0, 2.0, 3.0));
}

#[test]
fn refuses_a_placement_that_cannot_hold_a_picture() {
    let png = png_header(200, 100);
    let ok = Placement::ground(2.0);
    for bad in [
        Placement { width: 0.0, ..ok },
        Placement { width: -1.0, ..ok },
        Placement {
            width: f64::NAN,
            ..ok
        },
        Placement {
            u: Vec3::new(0.0, 0.0, 0.0),
            ..ok
        },
        Placement {
            v: ok.u * 3.0,
            ..ok
        },
        Placement {
            origin: Point3::new(f64::INFINITY, 0.0, 0.0),
            ..ok
        },
    ] {
        assert!(
            matches!(
                import(&png, None, "x", &bad),
                Err(ImageError::BadPlacement(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        import(&png, Some(ImageFormat::Jpeg), "x", &ok),
        Err(ImageError::Unreadable(_))
    ));
}

/// The recipe is the `.skp` image shape: one textured quad in a
/// definition named after the picture, its tile the size of the face and
/// its frame stretching that tile over the face once, placed by one
/// instance whose pose is the placement frame.
#[test]
fn builds_one_painted_face_in_a_definition_placed_once() {
    let png = png_header(400, 300);
    let place = Placement {
        origin: Point3::new(1.0, 0.0, 0.5),
        u: Vec3::new(1.0, 0.0, 0.0),
        v: Vec3::new(0.0, 0.0, 1.0),
        width: 2.0,
    };
    let out = import(&png, Some(ImageFormat::Png), "Wall", &place).unwrap();
    assert_eq!((out.width, out.height), (2.0, 1.5));
    let scene = &out.scene;
    assert_eq!(scene.materials.len(), 1);
    let m = &scene.materials[0];
    assert_eq!(m.name, "Wall");
    assert_eq!(m.color, Rgba8::rgba(255, 255, 255, 255));
    let t = m.texture.as_ref().unwrap();
    assert_eq!(t.image, png, "bytes kept verbatim");
    assert_eq!(t.world_size, [2.0, 1.5]);
    assert_eq!(scene.defs.len(), 1);
    assert_eq!(scene.defs[0].name.as_deref(), Some("Wall"));
    let meshes: Vec<_> = scene.defs[0].mesh_recipes().collect();
    assert_eq!(meshes.len(), 1);
    let mesh = meshes[0];
    assert_eq!(mesh.faces, vec![vec![0, 1, 2, 3]]);
    let frame = mesh.face_uv_frames[0].unwrap();
    let uvs: Vec<[f64; 2]> = mesh.positions.iter().map(|&p| frame.apply(p)).collect();
    assert_eq!(uvs, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    assert_eq!(scene.roots.len(), 1);
    let ImportNode::Instance { def, pose, key, .. } = &scene.roots[0] else {
        panic!("the root is an instance");
    };
    assert_eq!((*def, *key), (0, Some(INSTANCE_KEY)));
    assert_eq!(*pose, place.pose().unwrap());
}

/// A picture too small for the kernel to build its face is refused up
/// front, naming the minimum, rather than reaching the document as a face
/// the import would skip.
#[test]
fn refuses_a_picture_too_small_for_its_face_to_build() {
    // 3:1, so a width just past the minimum gives a height under it.
    let png = png_header(300, 100);
    let tiny = Placement::ground(1.1e-6);
    let err = import(&png, None, "x", &tiny).err().expect("refused");
    assert!(matches!(err, ImageError::TooSmall { .. }), "{err:?}");
    assert_eq!(err.code(), "ImageTooSmall");
    assert!(err.to_string().contains("0.000001 m"), "{err}");
    // Both sides at the minimum place.
    let square = png_header(100, 100);
    let out = import(&square, None, "x", &Placement::ground(MIN_SIDE)).unwrap();
    assert_eq!((out.width, out.height), (MIN_SIDE, MIN_SIDE));
    // And the kernel builds that face: the minimum is on the safe side.
    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, Vec::new()).unwrap();
    assert!(report.skipped.is_empty(), "skipped at the minimum");
    assert!(report.defs[0].is_some());
}

/// A JPEG's frame header can say more than 64 megapixels too.
#[test]
fn refuses_a_jpeg_over_64_megapixels() {
    assert_eq!(
        read_info(&jpeg_header(10_000, 8_000, None, false)),
        Err(ImageError::TooLarge {
            width: 10_000,
            height: 8_000
        })
    );
    // Turned on its side, the refusal names the size as shown.
    assert_eq!(
        read_info(&jpeg_header(10_000, 8_000, Some(6), true)),
        Err(ImageError::TooLarge {
            width: 8_000,
            height: 10_000
        })
    );
}

/// A damaged Exif block reads as no orientation (the picture unturned),
/// never a panic or a walk past its end.
///
/// Honestly: these cases also pass on the reader without its checked
/// offsets and count cap, on a 64-bit or a 32-bit host, and no input can
/// make them fail there. Each offset is formed only after a read at a
/// smaller one succeeded (a huge directory offset fails its first read
/// before anything is added to it), so no offset strays more than
/// 12 x 65,535 + 10 bytes past a block that a JPEG segment caps at 64 KiB,
/// nowhere near overflowing either `usize`; and a read past the end was
/// already `None`. The checked math is defense in depth, and the cap
/// saves walking up to 65,535 entries past the block's end; what this
/// spec pins is the answer.
#[test]
fn a_damaged_exif_block_leaves_the_picture_unturned() {
    // A directory offset far past the block's end.
    let mut far = b"II".to_vec();
    far.extend_from_slice(&42u16.to_le_bytes());
    far.extend_from_slice(&0xffff_fff0u32.to_le_bytes());
    assert_eq!(exif_orientation(&far), None);
    // A directory claiming 65535 entries in a block holding one, which is
    // not the orientation.
    let mut huge = b"MM".to_vec();
    huge.extend_from_slice(&42u16.to_be_bytes());
    huge.extend_from_slice(&8u32.to_be_bytes());
    huge.extend_from_slice(&0xffffu16.to_be_bytes());
    huge.extend_from_slice(&0x0100u16.to_be_bytes());
    huge.extend_from_slice(&4u16.to_be_bytes());
    huge.extend_from_slice(&1u32.to_be_bytes());
    huge.extend_from_slice(&640u32.to_be_bytes());
    assert_eq!(exif_orientation(&huge), None);
    // An orientation entry the count reaches but the block cuts short.
    let mut cut = b"II".to_vec();
    cut.extend_from_slice(&42u16.to_le_bytes());
    cut.extend_from_slice(&8u32.to_le_bytes());
    cut.extend_from_slice(&1u16.to_le_bytes());
    cut.extend_from_slice(&0x0112u16.to_le_bytes());
    cut.extend_from_slice(&3u16.to_le_bytes());
    assert_eq!(exif_orientation(&cut), None);
    // A JPEG carrying the far-offset block reads at its stored size.
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
    let mut seg = b"Exif\0\0".to_vec();
    seg.extend_from_slice(&far);
    jpeg.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&seg);
    jpeg.extend_from_slice(&jpeg_header(400, 300, None, false)[2..]);
    let info = read_info(&jpeg).unwrap();
    assert_eq!((info.width, info.height), (400, 300));
}

/// `check` answers what the kernel's ingest would do with a picture's
/// recipe, before any document sees it: the recipe `import` returns
/// checks clean, and the same recipe shrunk past what the kernel can find
/// a plane for (10 × 3.3 nm: twice its area under 1e-12 m² and narrower
/// than `kernel::tol::MIN_FACE_WIDTH`) is refused typed (`ImageTooSmall`)
/// with the kernel's reason, the very reason ingest would skip the face
/// with. Shrunk to 0.1 × 0.033 µm, which the area floor alone used to
/// refuse, it is 50 nm wide and checks clean now.
#[test]
fn check_refuses_a_recipe_the_kernel_would_not_build() {
    let png = png_header(300, 100);
    let shrunk = |k: f64| {
        let mut out = import(&png, None, "x", &Placement::ground(1.0)).unwrap();
        let kernel::ImportNode::Mesh(mesh) = &mut out.scene.defs[0].children[0] else {
            panic!("the definition holds the picture's mesh");
        };
        for p in &mut mesh.positions {
            *p = Point3::new(p.x * k, p.y * k, 0.0);
        }
        out
    };
    let out = import(&png, None, "x", &Placement::ground(1.0)).unwrap();
    assert_eq!(check(&out.scene), Ok(()));
    assert_eq!(check(&shrunk(1e-7).scene), Ok(()), "50 nm wide: kept now");
    let out = shrunk(1e-8);
    let kernel::ImportNode::Mesh(mesh) = &out.scene.defs[0].children[0] else {
        panic!("the definition holds the picture's mesh");
    };
    let reason = mesh.check_builds().unwrap_err().to_string();
    let err = check(&out.scene).unwrap_err();
    assert_eq!(err, ImageError::Unbuildable(reason.clone()));
    assert_eq!(err.code(), "ImageTooSmall");
    assert!(err.to_string().contains(&reason), "{err}");
    let mut doc = kernel::Document::new();
    let (report, _) = doc.ingest(out.scene, Vec::new()).unwrap();
    assert_eq!(
        report.skipped[0].reason, reason,
        "ingest skips it for the same reason"
    );
}

/// A picture placed absurdly large is refused with its own typed error,
/// `ImageTooBig`, not the "too small" one: either side over [`MAX_SIDE`]
/// (width given, or the height that width implies), including a width so
/// large its height overflows.
#[test]
fn refuses_a_picture_too_big_with_its_own_error() {
    let png = png_header(300, 100);
    for width in [1e9, 1e200, f64::MAX] {
        let err = import(&png, None, "x", &Placement::ground(width))
            .err()
            .expect("refused");
        assert!(matches!(err, ImageError::TooBig { .. }), "{err:?}");
        assert_eq!(err.code(), "ImageTooBig");
        assert!(err.to_string().contains("at most 1000000 m"), "{err}");
    }
    // The height can be the long side: a tall picture just under the limit
    // wide is still too big.
    let tall = png_header(100, 300);
    let err = import(&tall, None, "x", &Placement::ground(5e5))
        .err()
        .expect("refused");
    assert!(
        matches!(err, ImageError::TooBig { width, height } if width == 5e5 && height == 1.5e6),
        "{err:?}"
    );
    // Both sides at the limit still place.
    let square = png_header(100, 100);
    let out = import(&square, None, "x", &Placement::ground(MAX_SIDE)).unwrap();
    assert_eq!((out.width, out.height), (MAX_SIDE, MAX_SIDE));
    // And the kernel builds that face.
    assert_eq!(check(&out.scene), Ok(()));
}
