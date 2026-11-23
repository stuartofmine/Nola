// SPDX-License-Identifier: Apache-2.0
use crate::config_paths::expand_home;
use crate::i18n::{current_language, parse_language_tag, pick_language, set_language, Language};
use nola_update_host::{HttpsTransport, UpdateSource};
use std::path::PathBuf;

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), String>> {
    run_if_requested_with_json(args, false)
}

pub(crate) fn run_if_requested_with_json(
    args: &[String],
    leading_json: bool,
) -> Option<Result<(), String>> {
    if args.first().map(String::as_str) != Some("update") {
        return None;
    }
    Some(run(args, leading_json).map_err(|e| e.to_string()))
}

fn resolve_download_dir(raw: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let expanded = expand_home(&PathBuf::from(raw));
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    })
}

fn localized_update_status(status: &'static str) -> &'static str {
    let values = match status {
        "available" => [
            "发现可用更新",
            "發現可用更新",
            "Update available",
            "Update verfügbar",
            "Actualización disponible",
            "Mise à jour disponible",
            "アップデートがあります",
            "업데이트 사용 가능",
            "Доступно обновление",
        ],
        "up_to_date" => [
            "已是最新版本",
            "已是最新版本",
            "Up to date",
            "Auf dem neuesten Stand",
            "Actualizado",
            "À jour",
            "最新です",
            "최신 버전입니다",
            "Установлена актуальная версия",
        ],
        "disabled" => [
            "未配置更新源",
            "未設定更新來源",
            "Update checks disabled (no update feed configured)",
            "Update-Prüfung deaktiviert (kein Update-Feed konfiguriert)",
            "Comprobación de actualizaciones deshabilitada (sin origen configurado)",
            "Vérification des mises à jour désactivée (aucun flux configuré)",
            "アップデート確認は無効です（更新フィード未設定）",
            "업데이트 확인이 비활성화됨(업데이트 피드 미설정)",
            "Проверка обновлений отключена (источник не настроен)",
        ],
        "ahead" => [
            "当前版本高于发布频道",
            "目前版本高於發佈頻道",
            "Installed build is ahead of the release channel",
            "Installierter Build ist neuer als der Release-Kanal",
            "La compilación instalada es más reciente que el canal de lanzamiento",
            "La version installée est plus récente que le canal de publication",
            "インストール済みビルドはリリースチャンネルより新しいです",
            "설치된 빌드가 릴리스 채널보다 최신입니다",
            "Установленная сборка новее канала обновлений",
        ],
        "unavailable" => [
            "当前平台暂无可用安装包",
            "目前平台暫無可用安裝套件",
            "No release package available for this target",
            "Kein Release-Paket für diese Plattform verfügbar",
            "No hay paquete de lanzamiento disponible para esta plataforma",
            "Aucun paquet disponible pour cette plateforme",
            "このプラットフォーム向けのパッケージはありません",
            "이 플랫폼용 릴리스 패키지가 없습니다",
            "Для этой платформы нет доступного пакета",
        ],
        "incompatible_data" => [
            "新版本数据格式不兼容，需按升级说明迁移",
            "新版本資料格式不相容，需依升級說明遷移",
            "Release requires an incompatible data format migration",
            "Release erfordert eine inkompatible Datenformat-Migration",
            "La versión requiere una migración de formato de datos incompatible",
            "La version nécessite une migration de format de données incompatible",
            "このリリースには互換性のないデータ形式移行が必要です",
            "이 릴리스는 호환되지 않는 데이터 형식 마이그레이션이 필요합니다",
            "Выпуск требует несовместимой миграции формата данных",
        ],
        _ => return status,
    };
    pick_language(current_language(), values)
}

fn format_check_human(checked: &nola_update_host::Check) -> String {
    let status_text = localized_update_status(checked.status);
    let mut lines = vec![match current_language() {
        Language::ZhHans => format!(
            "更新状态：{status_text}（当前版本 {}，构建 {}，频道 {}）",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::ZhHant => format!(
            "更新狀態：{status_text}（目前版本 {}，建置 {}，頻道 {}）",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::De => format!(
            "Update-Status: {status_text} (aktuell {} Build {}, Kanal {})",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::Es => format!(
            "Estado de actualización: {status_text} (actual {} compilación {}, canal {})",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::Fr => format!(
            "État de mise à jour : {status_text} (actuel {} build {}, canal {})",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::Ja => format!(
            "更新状態: {status_text}（現在 {} ビルド {}、チャンネル {}）",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::Ko => format!(
            "업데이트 상태: {status_text} (현재 {} 빌드 {}, 채널 {})",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::Ru => format!(
            "Статус обновления: {status_text} (текущая {} сборка {}, канал {})",
            checked.current_version, checked.current_build, checked.channel
        ),
        Language::En => format!(
            "Update status: {status_text} (current {} build {}, channel {})",
            checked.current_version, checked.current_build, checked.channel
        ),
    }];
    if let Some(offer) = &checked.release {
        lines.push(match current_language() {
            Language::ZhHans => format!("可用版本：{}（构建 {}）", offer.version, offer.build),
            Language::ZhHant => format!("可用版本：{}（建置 {}）", offer.version, offer.build),
            Language::De => format!("Verfügbare Version: {} (Build {})", offer.version, offer.build),
            Language::Es => format!("Versión disponible: {} (compilación {})", offer.version, offer.build),
            Language::Fr => format!("Version disponible : {} (build {})", offer.version, offer.build),
            Language::Ja => format!("利用可能なバージョン: {}（ビルド {}）", offer.version, offer.build),
            Language::Ko => format!("사용 가능한 버전: {} (빌드 {})", offer.version, offer.build),
            Language::Ru => format!("Доступная версия: {} (сборка {})", offer.version, offer.build),
            Language::En => format!("Available release: {} (build {})", offer.version, offer.build),
        });
        let notes = offer.notes.trim();
        if !notes.is_empty() {
            lines.push(notes.to_string());
        }
    }
    lines.join("\n")
}

fn no_downloadable_release_error() -> &'static str {
    pick_language(
        current_language(),
        [
            "没有可下载的发布版本。请先检查更新状态和发布源配置。",
            "沒有可下載的發佈版本。請先檢查更新狀態與發佈來源設定。",
            "No downloadable release. Check status and publisher configuration first.",
            "Kein herunterladbares Release verfügbar. Prüfen Sie zuerst Status und Herausgeber-Konfiguration.",
            "No hay ninguna versión descargable. Comprueba primero el estado y la configuración del publicador.",
            "Aucune version téléchargeable. Vérifiez d’abord l’état et la configuration de publication.",
            "ダウンロード可能なリリースがありません。先にステータスと公開設定を確認してください。",
            "다운로드할 수 있는 릴리스가 없습니다. 먼저 상태와 게시자 구성을 확인하세요.",
            "Нет доступного для загрузки выпуска. Сначала проверьте статус и настройки источника.",
        ],
    )
}

fn format_download_human(path: &std::path::Path, version: &str, build: u64) -> String {
    let display_path = path.display();
    match current_language() {
        Language::ZhHans => format!(
            "已下载 Nola {version}（构建 {build}）到 {display_path}\n下一步：请先显式停止受影响的 Nola Host 及其活动任务，再安装完整的匹配发布包。当前未替换任何可执行文件或用户数据。"
        ),
        Language::ZhHant => format!(
            "已下載 Nola {version}（建置 {build}）至 {display_path}\n下一步：請先明確停止受影響的 Nola Host 及其活動任務，再安裝完整的對應發佈套件。目前未取代任何執行檔或使用者資料。"
        ),
        Language::De => format!(
            "Nola {version} (Build {build}) heruntergeladen nach {display_path}\nNächster Schritt: Stoppen Sie den betroffenen Nola Host und seine aktiven Aufgaben ausdrücklich und installieren Sie dann das vollständige passende Release-Paket. Es wurden weder Programme noch Benutzerdaten ersetzt."
        ),
        Language::Es => format!(
            "Descargado Nola {version} (compilación {build}) en {display_path}\nSiguiente paso: detén explícitamente el Nola Host afectado y sus tareas activas, luego instala el paquete completo correspondiente. No se ha reemplazado ningún ejecutable ni dato de usuario."
        ),
        Language::Fr => format!(
            "Nola {version} (build {build}) téléchargé dans {display_path}\nÉtape suivante : arrêtez explicitement le Nola Host concerné et ses tâches actives, puis installez le paquet complet correspondant. Aucun exécutable ni donnée utilisateur n’a été remplacé."
        ),
        Language::Ja => format!(
            "Nola {version}（ビルド {build}）を {display_path} にダウンロードしました\n次の手順：対象の Nola Host と実行中タスクを明示的に停止してから、対応する完全なリリースパッケージをインストールしてください。実行ファイルやユーザーデータは置換されていません。"
        ),
        Language::Ko => format!(
            "Nola {version} (빌드 {build})을(를) {display_path}에 다운로드했습니다\n다음 단계: 영향을 받는 Nola Host와 활성 작업을 명시적으로 중지한 다음 일치하는 전체 릴리스 번들을 설치하세요. 실행 파일이나 사용자 데이터는 교체되지 않았습니다."
        ),
        Language::Ru => format!(
            "Загружена версия Nola {version} (сборка {build}) в {display_path}\nСледующий шаг: явно остановите затронутый Nola Host и его активные задачи, затем установите полный соответствующий пакет релиза. Исполняемые файлы и пользовательские данные не заменялись."
        ),
        Language::En => format!(
            "Downloaded Nola {version} (build {build}) to {display_path}\nNext step: Stop the affected Nola Host and its active tasks explicitly, then install the complete matching release bundle. No executable or user data has been replaced."
        ),
    }
}

fn run(args: &[String], leading_json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let rest = args[1..].to_vec();
    if rest
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("Usage:\n  nola update check [--source <absolute-public-source-json>] [--json]\n  nola update download <directory> [--source <absolute-public-source-json>] [--json]");
        return Ok(());
    }
    let mut json_output = leading_json;
    let mut source_path: Option<PathBuf> = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < rest.len() {
        let arg = &rest[index];
        if arg == "--" {
            positional.extend(rest[index + 1..].iter().cloned());
            break;
        }
        if arg == "--json" {
            json_output = true;
            index += 1;
        } else if arg == "--lang" {
            let value = rest
                .get(index + 1)
                .ok_or("--lang requires a language tag")?;
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--lang=") {
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 1;
        } else if arg == "--source" {
            let value = rest
                .get(index + 1)
                .ok_or("--source requires an absolute public source JSON path")?;
            source_path = Some(PathBuf::from(value));
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--source=") {
            source_path = Some(PathBuf::from(value));
            index += 1;
        } else {
            positional.push(arg.clone());
            index += 1;
        }
    }
    let source = if let Some(path) = source_path {
        if !path.is_absolute() {
            return Err("--source must be absolute".into());
        }
        let mut source = UpdateSource::parse(&std::fs::read_to_string(path)?)?;
        let installed = UpdateSource::bundled()?;
        // A public source override cannot claim that another build is installed.
        source.version = installed.version;
        source.build = installed.build;
        source
    } else {
        UpdateSource::bundled()?
    };
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    if !matches!(refs.as_slice(), ["check"] | ["download", _]) {
        return Err("Usage: nola update check | download <directory> [--source <absolute-public-source-json>]".into());
    }
    let target = nola_update_host::native_target("cli");
    let checked = nola_update_host::check(&source, &target, &HttpsTransport)?;
    match refs.as_slice() {
        ["check"] => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&checked)?);
            } else {
                println!("{}", format_check_human(&checked));
            }
        }
        ["download", directory] => {
            let download_dir = resolve_download_dir(directory)?;
            let offer = checked
                .release
                .ok_or_else(no_downloadable_release_error)?;
            let package = nola_update_host::download(
                &source,
                &target,
                &offer.release_id,
                &download_dir,
                &HttpsTransport,
            )?;
            if json_output {
                println!(
                    "{}",
                    serde_json::json!({
                        "path": package,
                        "version": offer.version,
                        "build": offer.build,
                        "next_step": "Stop the affected Nola Host and its active tasks explicitly, then install the complete matching release bundle. No executable or user data has been replaced."
                    })
                );
            } else {
                println!("{}", format_download_human(&package, &offer.version, offer.build));
            }
        }
        _ => {
            return Err("Usage: nola update check | download <directory> [--source <absolute-public-source-json>]".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn explicit_command_is_independent_of_model_configuration() {
        assert!(run_if_requested(&["ordinary update discussion".into()]).is_none());
        let mut source = UpdateSource::bundled().unwrap();
        source.feed_url = None; source.public_key = None;
        let temporary = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temporary.path(), serde_json::to_vec(&source).unwrap()).unwrap();
        assert!(run_if_requested(&["update".into(),"check".into(),"--source".into(),temporary.path().to_string_lossy().into_owned()]).unwrap().is_ok());
        assert!(run_if_requested(&["update".into(),"check".into(),"--json".into(),"--source".into(),temporary.path().to_string_lossy().into_owned()]).unwrap().is_ok());
        assert!(run_if_requested(&["update".into(),"unknown".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"check".into(),"--source".into(),"relative".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"check".into(),"--source=relative".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"--help".into()]).unwrap().is_ok());
        assert!(run_if_requested(&["version".into()]).is_none());
        let resolved = resolve_download_dir("dist/updates").unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("dist/updates"));
    }
}
