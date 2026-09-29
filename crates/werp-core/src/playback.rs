//! Pure selection of the least preparation needed for a receiver profile.
use crate::{
    WerpError,
    media::{self, DirectPlayPolicy, MediaInfo},
    transcode::{self, TranscodeMode},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Auto,
    Direct,
    Remux,
    Audio,
    Transcode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Auto,
    Baseline,
    Extended,
    /// Explicit testing only; permits AC-3 passthrough, unlike Extended.
    Experimental,
}

impl Profile {
    /// Explicit profiles bypass both bundled and user-supplied model rules.
    pub fn resolve_with(
        self,
        model: Option<&str>,
        database: &crate::devices::Database,
    ) -> DirectPlayPolicy {
        if self == Self::Auto {
            database.policy(model)
        } else {
            self.resolve(model)
        }
    }

    pub fn resolve(self, model: Option<&str>) -> DirectPlayPolicy {
        match self {
            Self::Baseline => DirectPlayPolicy::Conservative,
            Self::Extended => DirectPlayPolicy::Extended,
            Self::Experimental => DirectPlayPolicy::Experimental,
            Self::Auto => crate::devices::policy(model),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    pub policy: DirectPlayPolicy,
    pub reason: &'static str,
}

impl Plan {
    pub fn preparation(self) -> Option<TranscodeMode> {
        match self.mode {
            Mode::Direct => None,
            Mode::Remux => Some(TranscodeMode::Remux),
            Mode::Audio => Some(TranscodeMode::AudioOnly),
            Mode::Transcode => Some(TranscodeMode::AudioVideo),
            Mode::Auto => unreachable!("selection always resolves automatic mode"),
        }
    }
}

pub fn select(info: &MediaInfo, mode: Mode, policy: DirectPlayPolicy) -> Result<Plan, WerpError> {
    // Ambiguous tracks and known HDR must never silently choose a destructive fallback.
    transcode::validate_input(info)?;
    let direct = || media::assess_direct_play(info, policy).map(|_| ());
    let remux = || media::validate_remux_input(info, policy);
    let audio = || media::validate_audio_transcode_input(info, policy);
    let selected = match mode {
        Mode::Auto if direct().is_ok() => Mode::Direct,
        Mode::Auto if remux().is_ok() => Mode::Remux,
        Mode::Auto if audio().is_ok() => Mode::Audio,
        Mode::Auto => Mode::Transcode,
        Mode::Direct => {
            direct()?;
            Mode::Direct
        }
        Mode::Remux => {
            remux()?;
            Mode::Remux
        }
        Mode::Audio => {
            audio()?;
            Mode::Audio
        }
        Mode::Transcode => Mode::Transcode,
    };
    Ok(Plan {
        mode: selected,
        policy: if selected == Mode::Transcode {
            DirectPlayPolicy::Conservative
        } else {
            policy
        },
        reason: match selected {
            Mode::Direct => "source matches receiver profile; serving original file",
            Mode::Remux => "copying supported video and audio into MP4",
            Mode::Audio => "preserving video and preparing stereo AAC in MP4",
            Mode::Transcode => "preparing H.264 and stereo AAC for compatible playback",
            Mode::Auto => unreachable!(),
        },
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn info(fixture: &str, change: impl FnOnce(&mut serde_json::Value)) -> MediaInfo {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        std::fs::write(&path, b"source").unwrap();
        let mut json: serde_json::Value = serde_json::from_str(fixture).unwrap();
        change(&mut json);
        let probe = dir.path().join("probe");
        std::fs::write(&probe, "#!/bin/sh\ncat \"$0.json\"\n").unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.path().join("probe.json"), json.to_string()).unwrap();
        media::inspect(
            &path,
            &media::ProbeOptions {
                executable: probe,
                ..Default::default()
            },
            &crate::CancellationToken::new(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn database_profile_uses_observed_limits_and_preserves_overrides() {
        let preferences = crate::devices::database();
        for model in ["KPN DIW7022", "DIW7022", " kpn diw7022 "] {
            let policy = Profile::Auto.resolve_with(Some(model), preferences);
            assert_eq!(policy.h264_max_fps(), 50);
            for (fps, expected) in [
                ("50/1", Mode::Direct),
                ("60000/1001", Mode::Transcode),
                ("60/1", Mode::Transcode),
            ] {
                let source = info(include_str!("../tests/fixtures/h264.json"), |j| {
                    j["streams"][0]["level"] = 42.into();
                    j["streams"][0]["r_frame_rate"] = fps.into();
                })
                .await;
                assert_eq!(select(&source, Mode::Auto, policy).unwrap().mode, expected);
                let opted_in = Profile::Auto.resolve_with(
                    Some(model),
                    &preferences.with_overrides("schema_version=1\n[[devices]]\nid='KPN DIW7022'\n[devices.playback]\nh264_max_fps=60").unwrap(),
                );
                assert_eq!(opted_in.h264_max_fps(), 60);
                assert_eq!(
                    select(&source, Mode::Auto, opted_in).unwrap().mode,
                    Mode::Direct
                );
                assert_eq!(
                    select(
                        &source,
                        Mode::Auto,
                        Profile::Baseline.resolve_with(Some(model), preferences)
                    )
                    .unwrap()
                    .mode,
                    Mode::Transcode
                );
                assert_eq!(
                    select(
                        &source,
                        Mode::Auto,
                        Profile::Auto.resolve_with(Some("Unknown TV"), preferences)
                    )
                    .unwrap()
                    .mode,
                    Mode::Transcode
                );
            }
            let ac3 = info(include_str!("../tests/fixtures/h264.json"), |j| {
                j["streams"][1]["codec_name"] = "ac3".into();
            })
            .await;
            assert_eq!(select(&ac3, Mode::Auto, policy).unwrap().mode, Mode::Audio);
            assert!(select(&ac3, Mode::Direct, policy).is_err());
        }
    }

    #[tokio::test]
    async fn high_frame_rate_h264_is_opt_in_and_keeps_other_limits() {
        let db = crate::devices::database().with_overrides("schema_version=1\n[[devices]]\nid='Test TV'\n[devices.playback]\nh264_max_level=42\nh264_max_fps=60").unwrap();
        let preferences = &db;
        let policy = Profile::Auto.resolve_with(Some("Test TV"), preferences);
        assert!(policy.allows_h264_high_frame_rate());
        assert!(!policy.allows_hevc());
        assert!(!policy.allows_aac_surround());
        for fps in ["25/1", "50/1", "60000/1001", "60/1"] {
            let source = info(include_str!("../tests/fixtures/h264.json"), |j| {
                j["streams"][0]["level"] = 42.into();
                j["streams"][0]["r_frame_rate"] = fps.into();
            })
            .await;
            assert_eq!(
                select(&source, Mode::Auto, policy).unwrap().mode,
                Mode::Direct
            );
            for explicit in [Profile::Baseline, Profile::Extended, Profile::Experimental] {
                assert_eq!(
                    select(
                        &source,
                        Mode::Auto,
                        explicit.resolve_with(Some("Test TV"), preferences)
                    )
                    .unwrap()
                    .mode,
                    Mode::Transcode
                );
            }
            assert_eq!(
                select(&source, Mode::Transcode, policy).unwrap().policy,
                DirectPlayPolicy::Conservative
            );
        }
        for (container, audio, expected) in [
            ("matroska", "aac", Mode::Remux),
            ("mp4", "ac3", Mode::Audio),
            ("matroska", "eac3", Mode::Audio),
        ] {
            let source = info(include_str!("../tests/fixtures/h264.json"), |j| {
                j["format"]["format_name"] = container.into();
                j["streams"][0]["level"] = 42.into();
                j["streams"][0]["r_frame_rate"] = "50/1".into();
                j["streams"][1]["codec_name"] = audio.into();
            })
            .await;
            assert_eq!(select(&source, Mode::Auto, policy).unwrap().mode, expected);
        }
        for (pointer, value) in [
            ("/streams/0/level", serde_json::json!(50)),
            ("/streams/0/r_frame_rate", serde_json::json!("120/1")),
            ("/streams/0/pix_fmt", serde_json::json!("yuv420p10le")),
            ("/streams/0/width", serde_json::json!(3840)),
            ("/streams/0/profile", serde_json::json!("High 10")),
            ("/streams/0/level", serde_json::Value::Null),
        ] {
            let source = info(include_str!("../tests/fixtures/h264.json"), |j| {
                *j.pointer_mut(pointer).unwrap() = value;
            })
            .await;
            assert_eq!(
                select(&source, Mode::Auto, policy).unwrap().mode,
                Mode::Transcode
            );
        }
        let relaxed = DirectPlayPolicy::H264HighFrameRate {
            allow_hevc: true,
            allow_aac_surround: true,
            max_fps: 60,
        };
        assert!(
            relaxed.allows_hevc()
                && relaxed.allows_aac_surround()
                && relaxed.allows_h264_high_frame_rate()
        );
        let hevc = info(include_str!("../tests/fixtures/hevc.json"), |j| {
            j["streams"][0]["r_frame_rate"] = "50/1".into();
        })
        .await;
        assert_eq!(
            select(&hevc, Mode::Auto, relaxed).unwrap().mode,
            Mode::Transcode
        );
    }

    #[tokio::test]
    async fn independent_model_permissions_and_explicit_profile_precedence() {
        for allow_hevc in [false, true] {
            for allow_aac_surround in [false, true] {
                let db = crate::devices::database().with_overrides(&format!("schema_version=1\n[[devices]]\nid='Test TV'\n[devices.playback]\nallow_hevc={allow_hevc}\nallow_aac_surround={allow_aac_surround}")).unwrap();
                let preferences = &db;
                let policy = Profile::Auto.resolve_with(Some("Test TV"), preferences);
                assert_eq!(policy.allows_hevc(), allow_hevc);
                assert_eq!(policy.allows_aac_surround(), allow_aac_surround);
                assert_eq!(
                    Profile::Baseline.resolve_with(Some("Test TV"), preferences),
                    DirectPlayPolicy::Conservative
                );
                assert_eq!(
                    Profile::Extended.resolve_with(Some("Test TV"), preferences),
                    DirectPlayPolicy::Extended
                );
                assert_eq!(
                    Profile::Auto.resolve_with(Some("KPN DIW7022"), preferences),
                    crate::devices::policy(Some("KPN DIW7022"))
                );
                for container in ["mp4", "matroska,webm"] {
                    for fixture in [
                        include_str!("../tests/fixtures/hevc.json"),
                        include_str!("../tests/fixtures/h264.json"),
                    ] {
                        for audio in ["aac", "ac3", "eac3"] {
                            let source = info(fixture, |j| {
                                j["format"]["format_name"] = container.into();
                                j["streams"][1]["codec_name"] = audio.into();
                                j["streams"][1]["channels"] = 6.into();
                            })
                            .await;
                            let is_hevc = source.streams[0].codec.as_deref() == Some("hevc");
                            let expected = if is_hevc && !allow_hevc {
                                Mode::Transcode
                            } else if audio != "aac" || !allow_aac_surround {
                                Mode::Audio
                            } else if container == "mp4" {
                                Mode::Direct
                            } else {
                                Mode::Remux
                            };
                            assert_eq!(select(&source, Mode::Auto, policy).unwrap().mode, expected);
                            assert_eq!(
                                select(&source, Mode::Transcode, policy).unwrap().policy,
                                DirectPlayPolicy::Conservative
                            );
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auto_chooses_minimum_conversion_and_overrides_obey_profile() {
        let h264 = include_str!("../tests/fixtures/h264.json");
        let hevc = include_str!("../tests/fixtures/hevc.json");
        for (fixture, container, codec, channels, baseline, extended) in [
            (h264, "mp4", "aac", 2, Mode::Direct, Mode::Direct),
            (h264, "matroska,webm", "aac", 2, Mode::Remux, Mode::Remux),
            (h264, "matroska,webm", "aac", 6, Mode::Audio, Mode::Remux),
            (h264, "matroska,webm", "eac3", 6, Mode::Audio, Mode::Audio),
            (h264, "mp4", "ac3", 6, Mode::Audio, Mode::Audio),
            (hevc, "mp4", "aac", 6, Mode::Transcode, Mode::Direct),
            (
                hevc,
                "matroska,webm",
                "aac",
                6,
                Mode::Transcode,
                Mode::Remux,
            ),
            (
                hevc,
                "matroska,webm",
                "eac3",
                6,
                Mode::Transcode,
                Mode::Audio,
            ),
        ] {
            let source = info(fixture, |j| {
                j["format"]["format_name"] = container.into();
                j["streams"][1]["codec_name"] = codec.into();
                j["streams"][1]["channels"] = channels.into();
            })
            .await;
            for (policy, expected) in [
                (DirectPlayPolicy::Conservative, baseline),
                (DirectPlayPolicy::Extended, extended),
            ] {
                assert_eq!(select(&source, Mode::Auto, policy).unwrap().mode, expected);
                assert_eq!(
                    select(&source, Mode::Transcode, policy).unwrap().policy,
                    DirectPlayPolicy::Conservative
                );
                if expected != Mode::Direct {
                    assert!(select(&source, Mode::Direct, policy).is_err());
                }
            }
        }
        let he_aac = info(hevc, |j| {
            j["format"]["format_name"] = "matroska,webm".into();
            j["streams"][1]["profile"] = "HE-AAC".into();
        })
        .await;
        assert_eq!(
            select(&he_aac, Mode::Auto, DirectPlayPolicy::Extended)
                .unwrap()
                .mode,
            Mode::Audio
        );
        let hdr = info(hevc, |j| {
            j["streams"][0]["color_transfer"] = "smpte2084".into()
        })
        .await;
        assert!(select(&hdr, Mode::Auto, DirectPlayPolicy::Extended).is_err());
        let ambiguous = info(h264, |j| {
            let audio = j["streams"][1].clone();
            j["streams"].as_array_mut().unwrap().push(audio);
        })
        .await;
        assert!(select(&ambiguous, Mode::Auto, DirectPlayPolicy::Extended).is_err());
        assert_eq!(
            Profile::Auto.resolve(Some("KPN DIW7022")),
            crate::devices::policy(Some("KPN DIW7022"))
        );
        assert_eq!(
            Profile::Auto.resolve(Some("Unknown receiver")),
            DirectPlayPolicy::Conservative
        );
        assert_eq!(Profile::Auto.resolve(None), DirectPlayPolicy::Conservative);
    }
}
