use super::*;

#[test]
fn detection_accepts_every_kimi_code_video_suffix() {
    for (filename, expected_mime) in [
        ("sample.mp4", "video/mp4"),
        ("sample.mpg", "video/mpeg"),
        ("sample.mpeg", "video/mpeg"),
        ("sample.mkv", "video/x-matroska"),
        ("sample.avi", "video/x-msvideo"),
        ("sample.mov", "video/quicktime"),
        ("sample.ogv", "video/ogg"),
        ("sample.wmv", "video/x-ms-wmv"),
        ("sample.webm", "video/webm"),
        ("sample.m4v", "video/x-m4v"),
        ("sample.flv", "video/x-flv"),
        ("sample.3gp", "video/3gpp"),
        ("sample.3g2", "video/3gpp2"),
    ] {
        assert_eq!(
            mime_type(Path::new(filename), b"no recognizable magic"),
            Some(expected_mime),
            "unexpected MIME for {filename}"
        );
    }
}

#[test]
fn detection_prefers_mp4_magic_over_the_suffix() {
    let mut header = vec![0, 0, 0, 24];
    header.extend_from_slice(b"ftypisom");

    assert_eq!(
        mime_type(Path::new("sample.bin"), &header),
        Some("video/mp4")
    );
}

#[test]
fn multipart_body_matches_the_expected_file_and_purpose_parts() {
    let body = multipart_body("boundary", "probe.mp4", "video/mp4", b"video-bytes");
    let body = String::from_utf8(body).expect("multipart fixture is UTF-8");

    assert_eq!(
        body,
        "--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"probe.mp4\"\r\nContent-Type: video/mp4\r\n\r\nvideo-bytes\r\n--boundary\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nvideo\r\n--boundary--\r\n"
    );
}
