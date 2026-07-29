use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use crate::CommandError;

const MAX_UPDATE_VERSION_LENGTH: usize = 80;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplicationUpdateInfo {
    version: String,
    body: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplicationUpdateCheckResult {
    current_version: String,
    update: Option<ApplicationUpdateInfo>,
}

fn validate_expected_version(version: &str) -> Result<&str, CommandError> {
    let trimmed = version.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_UPDATE_VERSION_LENGTH
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+_".contains(character))
    {
        return Err(CommandError {
            message: "the requested application update version is invalid".to_owned(),
        });
    }
    Ok(trimmed)
}

fn updater_error(
    message: &'static str,
) -> impl FnOnce(tauri_plugin_updater::Error) -> CommandError {
    move |_| CommandError {
        message: message.to_owned(),
    }
}

#[tauri::command]
pub(crate) async fn application_update_check(
    app: AppHandle,
) -> Result<ApplicationUpdateCheckResult, CommandError> {
    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(updater_error(
            "the application updater could not be initialized",
        ))?
        .check()
        .await
        .map_err(updater_error("the update service could not be reached"))?
        .map(|update| ApplicationUpdateInfo {
            version: update.version,
            body: update.body,
            date: update.date.map(|date| date.to_string()),
        });

    Ok(ApplicationUpdateCheckResult {
        current_version,
        update,
    })
}

#[tauri::command]
pub(crate) async fn application_update_install(
    app: AppHandle,
    expected_version: String,
) -> Result<(), CommandError> {
    let expected_version = validate_expected_version(&expected_version)?;
    let update = app
        .updater()
        .map_err(updater_error(
            "the application updater could not be initialized",
        ))?
        .check()
        .await
        .map_err(updater_error("the update service could not be reached"))?
        .ok_or_else(|| CommandError {
            message: "the selected application update is no longer available".to_owned(),
        })?;

    if update.version != expected_version {
        return Err(CommandError {
            message:
                "a different application update is now available; check again before installing"
                    .to_owned(),
        });
    }

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(updater_error(
            "the application update could not be downloaded or installed",
        ))?;
    Ok(())
}

#[tauri::command]
pub(crate) fn application_update_restart(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_update_version_is_bounded_and_plain() {
        assert_eq!(validate_expected_version("1.2.3").unwrap(), "1.2.3");
        assert_eq!(
            validate_expected_version(" app-v1.2.3-beta+5 ").unwrap(),
            "app-v1.2.3-beta+5"
        );
        assert!(validate_expected_version("").is_err());
        assert!(validate_expected_version("../release").is_err());
        assert!(validate_expected_version("1.2.3\nnext").is_err());
        assert!(validate_expected_version(&"a".repeat(81)).is_err());
    }
}
