use yeet_core::{YeetError, discovery};
#[derive(Clone)]
pub enum Target {
    Auto,
    Device(String),
    Host(std::net::SocketAddr),
}
#[cfg(test)]
fn select_target(
    devices: &[discovery::Device],
    target: &Target,
) -> Result<discovery::Device, YeetError> {
    select_preferred_target(devices, target, &[])
}

pub fn select_preferred_target(
    devices: &[discovery::Device],
    target: &Target,
    preferred: &[String],
) -> Result<discovery::Device, YeetError> {
    let result = (|| {
        if matches!(target, Target::Auto) {
            let eligible: Vec<_> = devices
                .iter()
                .filter(|d| {
                    d.capabilities.is_some_and(|bits| bits & 1 != 0) && !d.addresses.is_empty()
                })
                .cloned()
                .collect();
            for preference in preferred {
                if eligible
                    .iter()
                    .any(|d| d.id == *preference || d.name == *preference)
                {
                    return discovery::select(&eligible, preference);
                }
            }
        }
        select_target_inner(devices, target)
    })();
    result.map_err(|error| match error {
        YeetError::Discovery(message) => YeetError::Discovery(format!(
            "{message}. Detected devices: {}",
            if devices.is_empty() {
                "none".into()
            } else {
                devices
                    .iter()
                    .map(|d| {
                        format!(
                            "{:?} ({}){}",
                            d.name,
                            d.id,
                            if d.capabilities.is_some_and(|c| c & 1 == 0) {
                                " [audio-only]"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        other => other,
    })
}

fn select_target_inner(
    devices: &[discovery::Device],
    target: &Target,
) -> Result<discovery::Device, YeetError> {
    let selected = match target {
        Target::Device(selector) => discovery::select(devices, selector)?,
        Target::Auto => {
            let eligible: Vec<_> = devices
                .iter()
                .filter(|d| {
                    d.capabilities.is_some_and(|bits| bits & 1 != 0) && !d.addresses.is_empty()
                })
                .collect();
            match eligible.as_slice() {
                [device] => (*device).clone(),
                _ => {
                    return Err(YeetError::Discovery(format!(
                        "found {} confirmed video receivers; choose --device NAME/ID or --host IP",
                        eligible.len(),
                    )));
                }
            }
        }
        Target::Host(_) => {
            return Err(YeetError::Discovery(
                "explicit IP does not require discovery".into(),
            ));
        }
    };
    if selected.capabilities.is_some_and(|bits| bits & 1 == 0) {
        return Err(YeetError::Discovery("selected device is audio-only".into()));
    }
    if selected.addresses.is_empty() {
        return Err(YeetError::Discovery(
            "selected device has no IPv4 address".into(),
        ));
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selection_requires_exactly_one_known_video_receiver() {
        let video = discovery::Device {
            id: "tv-id".into(),
            name: "TV".into(),
            model: "test".into(),
            addresses: vec!["127.0.0.1".parse().unwrap()],
            port: 8009,
            capabilities: Some(5),
        };
        let audio = discovery::Device {
            id: "speaker".into(),
            name: "Speaker".into(),
            capabilities: Some(4),
            ..video.clone()
        };
        let unknown = discovery::Device {
            id: "unknown".into(),
            name: "Unknown".into(),
            capabilities: None,
            ..video.clone()
        };
        let other = discovery::Device {
            id: "other-id".into(),
            name: "Other TV".into(),
            ..video.clone()
        };
        let devices = [video.clone(), other.clone(), audio.clone()];
        assert_eq!(
            select_preferred_target(
                &devices,
                &Target::Auto,
                &[
                    "Speaker".into(),
                    "missing".into(),
                    "Other TV".into(),
                    "TV".into()
                ]
            )
            .unwrap()
            .id,
            "other-id"
        );
        assert_eq!(
            select_preferred_target(&devices, &Target::Device("TV".into()), &["Other TV".into()])
                .unwrap()
                .id,
            "tv-id"
        );
        let error = select_preferred_target(&devices, &Target::Device("Missing".into()), &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Other TV")
                && error.contains("Speaker")
                && error.contains("[audio-only]")
        );
        let duplicate = discovery::Device {
            name: "TV".into(),
            ..other
        };
        assert!(
            select_preferred_target(&[video.clone(), duplicate], &Target::Auto, &["TV".into()])
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(select_target(&[], &Target::Auto).is_err());
        assert!(select_target(&[audio.clone(), unknown.clone()], &Target::Auto).is_err());
        assert_eq!(
            select_target(
                &[audio.clone(), unknown.clone(), video.clone()],
                &Target::Auto
            )
            .unwrap()
            .id,
            "tv-id"
        );
        assert!(
            select_target(
                &[
                    video.clone(),
                    discovery::Device {
                        id: "other-tv".into(),
                        ..video.clone()
                    }
                ],
                &Target::Auto
            )
            .is_err()
        );
        assert!(select_target(&[audio], &Target::Device("speaker".into())).is_err());
        assert!(select_target(&[unknown], &Target::Device("unknown".into())).is_ok());
        assert!(
            select_target(
                &[discovery::Device {
                    addresses: vec![],
                    ..video
                }],
                &Target::Device("tv-id".into())
            )
            .is_err()
        );
    }
}
