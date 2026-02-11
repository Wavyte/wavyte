#[cfg(feature = "media-ffmpeg")]
mod media_pipeline {
    use std::{collections::BTreeMap, path::Path, process::Command};

    use wavyte::{
        Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, FrameIndex,
        FrameRange, RenderSettings, RenderThreading, Track, Transform2D, VideoAsset,
        build_audio_manifest, create_backend, mix_manifest, render_frames_with_stats,
        render_to_mp4_with_stats,
    };

    fn ffmpeg_tools_available() -> bool {
        let ffmpeg_ok = Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let ffprobe_ok = Command::new("ffprobe")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ffmpeg_ok && ffprobe_ok
    }

    fn synth_media(root: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(root)?;

        let video_path = root.join("clip.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x64:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
            ])
            .arg(&video_path)
            .status()?;
        anyhow::ensure!(status.success(), "ffmpeg failed creating clip.mp4");

        let wav_path = root.join("tone.wav");
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=220:sample_rate=48000",
                "-t",
                "1",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&wav_path)
            .status()?;
        anyhow::ensure!(status.success(), "ffmpeg failed creating tone.wav");

        Ok(())
    }

    fn build_comp() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "v0".to_string(),
            Asset::Video(VideoAsset {
                source: "clip.mp4".to_string(),
                trim_start_sec: 0.10,
                trim_end_sec: Some(0.80),
                playback_rate: 1.25,
                volume: 0.8,
                fade_in_sec: 0.05,
                fade_out_sec: 0.05,
                muted: false,
            }),
        );
        assets.insert(
            "a0".to_string(),
            Asset::Audio(wavyte::AudioAsset {
                source: "tone.wav".to_string(),
                trim_start_sec: 0.0,
                trim_end_sec: Some(0.9),
                playback_rate: 1.0,
                volume: 0.5,
                fade_in_sec: 0.05,
                fade_out_sec: 0.05,
                muted: false,
            }),
        );

        let duration = FrameIndex(15);
        Composition {
            fps: wavyte::Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 64,
                height: 64,
            },
            duration,
            assets,
            tracks: vec![
                Track {
                    name: "video".to_string(),
                    z_base: 0,
                    layout_mode: wavyte::LayoutMode::Center,
                    layout_gap_px: 0.0,
                    layout_padding: wavyte::Edges::default(),
                    layout_align_x: wavyte::LayoutAlignX::Start,
                    layout_align_y: wavyte::LayoutAlignY::Start,
                    layout_grid_columns: 2,
                    clips: vec![Clip {
                        id: "c_video".to_string(),
                        asset: "v0".to_string(),
                        range: FrameRange::new(FrameIndex(0), duration).unwrap(),
                        props: ClipProps {
                            transform: Anim::constant(Transform2D::default()),
                            opacity: Anim::constant(1.0),
                            blend: BlendMode::Normal,
                        },
                        z_offset: 0,
                        effects: vec![],
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                Track {
                    name: "audio".to_string(),
                    z_base: 0,
                    layout_mode: wavyte::LayoutMode::Absolute,
                    layout_gap_px: 0.0,
                    layout_padding: wavyte::Edges::default(),
                    layout_align_x: wavyte::LayoutAlignX::Start,
                    layout_align_y: wavyte::LayoutAlignY::Start,
                    layout_grid_columns: 2,
                    clips: vec![Clip {
                        id: "c_audio".to_string(),
                        asset: "a0".to_string(),
                        range: FrameRange::new(FrameIndex(0), duration).unwrap(),
                        props: ClipProps {
                            transform: Anim::constant(Transform2D::default()),
                            opacity: Anim::constant(1.0),
                            blend: BlendMode::Normal,
                        },
                        z_offset: 0,
                        effects: vec![],
                        transition_in: None,
                        transition_out: None,
                    }],
                },
            ],
            seed: 1,
        }
    }

    #[test]
    fn video_render_is_consistent_between_sequential_and_parallel() {
        if !ffmpeg_tools_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "wavyte_media_pipeline_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        synth_media(&root).unwrap();

        let comp = build_comp();
        let range = FrameRange::new(FrameIndex(0), comp.duration).unwrap();
        let assets = wavyte::PreparedAssetStore::prepare(&comp, &root).unwrap();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };

        let mut seq_backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let (seq_frames, _) = render_frames_with_stats(
            &comp,
            range,
            seq_backend.as_mut(),
            &assets,
            &RenderThreading::default(),
        )
        .unwrap();

        let mut par_backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let opts = RenderThreading {
            parallel: true,
            chunk_size: 4,
            threads: Some(2),
            static_frame_elision: false,
        };
        let (par_frames, _) =
            render_frames_with_stats(&comp, range, par_backend.as_mut(), &assets, &opts).unwrap();
        assert_eq!(seq_frames.len(), par_frames.len());
        for (a, b) in seq_frames.iter().zip(par_frames.iter()) {
            assert_eq!(a.data, b.data);
        }
    }

    #[test]
    fn audio_manifest_and_mux_are_nonempty() {
        if !ffmpeg_tools_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "wavyte_media_mux_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        synth_media(&root).unwrap();

        let comp = build_comp();
        let range = FrameRange::new(FrameIndex(0), comp.duration).unwrap();
        let assets = wavyte::PreparedAssetStore::prepare(&comp, &root).unwrap();

        let manifest = build_audio_manifest(&comp, &assets, range).unwrap();
        assert!(!manifest.segments.is_empty());
        assert_eq!(
            manifest.total_samples,
            wavyte::frame_to_sample(comp.duration.0, comp.fps, manifest.sample_rate)
        );
        let mixed = mix_manifest(&manifest);
        assert!(mixed.iter().any(|v| v.abs() > 0.0));

        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };
        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let out = root.join("out_with_audio.mp4");
        let stats = render_to_mp4_with_stats(
            &comp,
            &out,
            wavyte::RenderToMp4Opts {
                range,
                bg_rgba: [0, 0, 0, 255],
                overwrite: true,
                threading: RenderThreading::default(),
            },
            backend.as_mut(),
            &assets,
        )
        .unwrap();
        assert_eq!(stats.frames_total, comp.duration.0);
        assert!(out.exists());
    }

    #[test]
    fn static_elision_still_works_when_audio_media_is_present() {
        if !ffmpeg_tools_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "wavyte_media_elision_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        synth_media(&root).unwrap();

        let mut comp = build_comp();
        // Use a static visual scene; audio asset is still present and mixed.
        comp.assets.insert(
            "p0".to_string(),
            Asset::Path(wavyte::PathAsset {
                svg_path_d: "M0,0 L64,0 L64,64 L0,64 Z".to_string(),
            }),
        );
        comp.tracks[0].clips[0].asset = "p0".to_string();
        comp.tracks[0].layout_mode = wavyte::LayoutMode::Absolute;

        let range = FrameRange::new(FrameIndex(0), comp.duration).unwrap();
        let assets = wavyte::PreparedAssetStore::prepare(&comp, &root).unwrap();
        let settings = RenderSettings {
            clear_rgba: Some([0, 0, 0, 255]),
        };
        let mut backend = create_backend(BackendKind::Cpu, &settings).unwrap();
        let opts = RenderThreading {
            parallel: true,
            chunk_size: comp.duration.0 as usize,
            threads: Some(2),
            static_frame_elision: true,
        };
        let (_, stats) =
            render_frames_with_stats(&comp, range, backend.as_mut(), &assets, &opts).unwrap();
        assert_eq!(stats.frames_rendered, 1);
        assert_eq!(stats.frames_elided, comp.duration.0 - 1);
    }
}
