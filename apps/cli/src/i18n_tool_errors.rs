// SPDX-License-Identifier: Apache-2.0

fn parse_tool_display_error(text: &str) -> Option<ToolDisplayError<'_>> {
    if let Some(name) = quoted_suffix(text, "Unknown tool '") {
        return Some(ToolDisplayError::UnknownTool(name));
    }
    if let Some(rest) = text.strip_prefix("Not a file: ") {
        return Some(ToolDisplayError::NotFile(rest));
    }
    if let Some(rest) = text.strip_prefix("Read failed: ") {
        return Some(ToolDisplayError::ReadFailed(rest));
    }
    if text == "content is required" {
        return Some(ToolDisplayError::ContentRequired);
    }
    if let Some(rest) = text
        .strip_prefix("content exceeds ")
        .and_then(|value| value.strip_suffix(" bytes"))
    {
        return Some(ToolDisplayError::ContentExceeds(rest));
    }
    if let Some(rest) = text.strip_prefix("Create parent directory failed: ") {
        return Some(ToolDisplayError::CreateParentFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("Write failed: ") {
        return Some(ToolDisplayError::WriteFailed(rest));
    }
    if text == "old_text is required" {
        return Some(ToolDisplayError::OldTextRequired);
    }
    if text == "new_text is required" {
        return Some(ToolDisplayError::NewTextRequired);
    }
    if text == "old_text was not found" {
        return Some(ToolDisplayError::OldTextNotFound);
    }
    if let Some(rest) = text
        .strip_prefix("old_text matched ")
        .and_then(|value| value.strip_suffix("; set replace_all=true or provide more context"))
    {
        return Some(ToolDisplayError::OldTextMatched(rest));
    }
    if let Some(rest) = text.strip_prefix("Not a directory: ") {
        return Some(ToolDisplayError::NotDirectory(rest));
    }
    if let Some(rest) = text.strip_prefix("List failed: ") {
        return Some(ToolDisplayError::ListFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("invalid regex: ") {
        return Some(ToolDisplayError::InvalidRegex(rest));
    }
    if text == "command too long" {
        return Some(ToolDisplayError::CommandTooLong);
    }
    if text == "shell metacharacters/substitutions are not allowed" {
        return Some(ToolDisplayError::ShellMetacharacters);
    }
    if text == "No bash prefixes whitelisted. Use --bash-prefix." {
        return Some(ToolDisplayError::NoBashPrefixes);
    }
    if let Some(head) = text
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix("' is not in the bash allow-list"))
    {
        return Some(ToolDisplayError::NotInBashAllowList(head));
    }
    if let Some(rest) = text.strip_prefix("cwd is not a directory: ") {
        return Some(ToolDisplayError::CwdNotDirectory(rest));
    }
    if let Some(rest) = text.strip_prefix("temp stdout failed: ") {
        return Some(ToolDisplayError::TempStdoutFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("temp stderr failed: ") {
        return Some(ToolDisplayError::TempStderrFailed(rest));
    }
    if let Some(rest) = text.strip_prefix("Exec failed: ") {
        return Some(ToolDisplayError::ExecFailed(rest));
    }
    if let Some(rest) = text
        .strip_prefix("Command timed out after ")
        .and_then(|value| value.strip_suffix('s'))
    {
        return Some(ToolDisplayError::CommandTimedOut(rest));
    }
    if let Some(rest) = text.strip_prefix("Wait failed: ") {
        return Some(ToolDisplayError::WaitFailed(rest));
    }
    if let Some(rest) = text.strip_suffix(" is required") {
        return Some(ToolDisplayError::Required(rest));
    }
    if text == "no allowed directories configured" {
        return Some(ToolDisplayError::NoAllowedDirs);
    }
    if let Some(rest) = text.strip_prefix("failed to resolve ") {
        return Some(ToolDisplayError::FailedResolve(rest));
    }
    if let Some(rest) = text.strip_prefix("path does not exist: ") {
        return Some(ToolDisplayError::PathDoesNotExist(rest));
    }
    if let Some(rest) = text.strip_prefix("failed to resolve parent for ") {
        return Some(ToolDisplayError::FailedResolveParent(rest));
    }
    if let Some(rest) = text.strip_prefix("path is outside allowed directories: ") {
        return Some(ToolDisplayError::OutsideAllowed(rest));
    }
    if let Some(rest) = text.strip_prefix("invalid glob pattern: ") {
        return Some(ToolDisplayError::InvalidGlob(rest));
    }
    None
}

fn format_tool_display_error(language: Language, error: ToolDisplayError<'_>) -> String {
    match language {
        Language::En => match error {
            ToolDisplayError::UnknownTool(name) => format!("Unknown tool '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Not a file: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Read failed: {err}"),
            ToolDisplayError::ContentRequired => "content is required".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content exceeds {size} bytes"),
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Create parent directory failed: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Write failed: {err}"),
            ToolDisplayError::OldTextRequired => "old_text is required".to_string(),
            ToolDisplayError::NewTextRequired => "new_text is required".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text was not found".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text matched {count} times; set replace_all=true or provide more context"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Not a directory: {path}"),
            ToolDisplayError::ListFailed(err) => format!("List failed: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("invalid regex: {err}"),
            ToolDisplayError::CommandTooLong => "command too long".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "shell metacharacters/substitutions are not allowed".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "No bash prefixes whitelisted. Use --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' is not in the bash allow-list")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd is not a directory: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => format!("temp stdout failed: {err}"),
            ToolDisplayError::TempStderrFailed(err) => format!("temp stderr failed: {err}"),
            ToolDisplayError::ExecFailed(err) => format!("Exec failed: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Command timed out after {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Wait failed: {err}"),
            ToolDisplayError::Required(key) => format!("{key} is required"),
            ToolDisplayError::NoAllowedDirs => "no allowed directories configured".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("failed to resolve {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("path does not exist: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("failed to resolve parent for {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("path is outside allowed directories: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("invalid glob pattern: {err}"),
        },
        Language::ZhHans => match error {
            ToolDisplayError::UnknownTool(name) => format!("未知工具 '{name}'"),
            ToolDisplayError::NotFile(path) => format!("不是文件：{path}"),
            ToolDisplayError::ReadFailed(err) => format!("读取失败：{err}"),
            ToolDisplayError::ContentRequired => "缺少 content".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content 超过 {size} 字节"),
            ToolDisplayError::CreateParentFailed(err) => format!("创建父目录失败：{err}"),
            ToolDisplayError::WriteFailed(err) => format!("写入失败：{err}"),
            ToolDisplayError::OldTextRequired => "缺少 old_text".to_string(),
            ToolDisplayError::NewTextRequired => "缺少 new_text".to_string(),
            ToolDisplayError::OldTextNotFound => "未找到 old_text".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text 匹配 {count} 次；请设置 replace_all=true 或提供更多上下文"
            ),
            ToolDisplayError::NotDirectory(path) => format!("不是目录：{path}"),
            ToolDisplayError::ListFailed(err) => format!("列出目录失败：{err}"),
            ToolDisplayError::InvalidRegex(err) => format!("正则表达式无效：{err}"),
            ToolDisplayError::CommandTooLong => "命令过长".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "不允许使用 shell 元字符或替换表达式".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "未配置允许的 Bash 前缀。请使用 --bash-prefix。".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' 不在 Bash 允许列表中")
            }
            ToolDisplayError::CwdNotDirectory(path) => format!("cwd 不是目录：{path}"),
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("创建临时 stdout 文件失败：{err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("创建临时 stderr 文件失败：{err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("执行失败：{err}"),
            ToolDisplayError::CommandTimedOut(sec) => format!("命令在 {sec}s 后超时"),
            ToolDisplayError::WaitFailed(err) => format!("等待命令结束失败：{err}"),
            ToolDisplayError::Required(key) => format!("缺少 {key}"),
            ToolDisplayError::NoAllowedDirs => "未配置允许访问的目录".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("解析路径失败 {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("路径不存在：{path}"),
            ToolDisplayError::FailedResolveParent(path) => format!("解析父目录失败：{path}"),
            ToolDisplayError::OutsideAllowed(path) => {
                format!("路径不在允许访问的目录内：{path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("glob 模式无效：{err}"),
        },
        Language::ZhHant => match error {
            ToolDisplayError::UnknownTool(name) => format!("未知工具 '{name}'"),
            ToolDisplayError::NotFile(path) => format!("不是檔案：{path}"),
            ToolDisplayError::ReadFailed(err) => format!("讀取失敗：{err}"),
            ToolDisplayError::ContentRequired => "缺少 content".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content 超過 {size} 位元組"),
            ToolDisplayError::CreateParentFailed(err) => format!("建立上層目錄失敗：{err}"),
            ToolDisplayError::WriteFailed(err) => format!("寫入失敗：{err}"),
            ToolDisplayError::OldTextRequired => "缺少 old_text".to_string(),
            ToolDisplayError::NewTextRequired => "缺少 new_text".to_string(),
            ToolDisplayError::OldTextNotFound => "找不到 old_text".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text 符合 {count} 次；請設定 replace_all=true 或提供更多上下文"
            ),
            ToolDisplayError::NotDirectory(path) => format!("不是目錄：{path}"),
            ToolDisplayError::ListFailed(err) => format!("列出目錄失敗：{err}"),
            ToolDisplayError::InvalidRegex(err) => format!("正規表示式無效：{err}"),
            ToolDisplayError::CommandTooLong => "命令過長".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "不允許使用 shell 中繼字元或替換表示式".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "未設定允許的 Bash 前綴。請使用 --bash-prefix。".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' 不在 Bash 允許清單中")
            }
            ToolDisplayError::CwdNotDirectory(path) => format!("cwd 不是目錄：{path}"),
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("建立暫存 stdout 檔案失敗：{err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("建立暫存 stderr 檔案失敗：{err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("執行失敗：{err}"),
            ToolDisplayError::CommandTimedOut(sec) => format!("命令在 {sec}s 後逾時"),
            ToolDisplayError::WaitFailed(err) => format!("等待命令結束失敗：{err}"),
            ToolDisplayError::Required(key) => format!("缺少 {key}"),
            ToolDisplayError::NoAllowedDirs => "未設定允許存取的目錄".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("解析路徑失敗 {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("路徑不存在：{path}"),
            ToolDisplayError::FailedResolveParent(path) => format!("解析上層目錄失敗：{path}"),
            ToolDisplayError::OutsideAllowed(path) => {
                format!("路徑不在允許存取的目錄內：{path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("glob 模式無效：{err}"),
        },
        Language::Ko => match error {
            ToolDisplayError::UnknownTool(name) => format!("알 수 없는 도구 '{name}'"),
            ToolDisplayError::NotFile(path) => format!("파일이 아닙니다: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("읽기 실패: {err}"),
            ToolDisplayError::ContentRequired => "content 가 필요합니다".to_string(),
            ToolDisplayError::ContentExceeds(size) => format!("content 가 {size} 바이트를 초과했습니다"),
            ToolDisplayError::CreateParentFailed(err) => format!("상위 디렉터리 생성 실패: {err}"),
            ToolDisplayError::WriteFailed(err) => format!("쓰기 실패: {err}"),
            ToolDisplayError::OldTextRequired => "old_text 가 필요합니다".to_string(),
            ToolDisplayError::NewTextRequired => "new_text 가 필요합니다".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text 를 찾을 수 없습니다".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text 가 {count}회 일치했습니다. replace_all=true 를 설정하거나 문맥을 더 지정하세요"
            ),
            ToolDisplayError::NotDirectory(path) => format!("디렉터리가 아닙니다: {path}"),
            ToolDisplayError::ListFailed(err) => format!("목록 조회 실패: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("잘못된 정규식: {err}"),
            ToolDisplayError::CommandTooLong => "명령이 너무 깁니다".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "셸 메타문자 또는 치환 구문은 허용되지 않습니다".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "허용된 Bash 접두사가 없습니다. --bash-prefix 를 사용하세요.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' 는 Bash 허용 목록에 없습니다")
            }
            ToolDisplayError::CwdNotDirectory(path) => format!("cwd 가 디렉터리가 아닙니다: {path}"),
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("임시 stdout 생성 실패: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("임시 stderr 생성 실패: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("실행 실패: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => format!("명령이 {sec}s 후 시간 초과되었습니다"),
            ToolDisplayError::WaitFailed(err) => format!("대기 실패: {err}"),
            ToolDisplayError::Required(key) => format!("{key} 가 필요합니다"),
            ToolDisplayError::NoAllowedDirs => "허용된 디렉터리가 설정되지 않았습니다".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("경로 확인 실패 {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("경로가 존재하지 않습니다: {path}"),
            ToolDisplayError::FailedResolveParent(path) => format!("상위 경로 확인 실패: {path}"),
            ToolDisplayError::OutsideAllowed(path) => {
                format!("경로가 허용된 디렉터리 밖에 있습니다: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("잘못된 glob 패턴: {err}"),
        },
        Language::De => match error {
            ToolDisplayError::UnknownTool(name) => format!("Unbekanntes Tool '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Keine Datei: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Lesen fehlgeschlagen: {err}"),
            ToolDisplayError::ContentRequired => "content ist erforderlich".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content überschreitet {size} Bytes")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Übergeordnetes Verzeichnis konnte nicht erstellt werden: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Schreiben fehlgeschlagen: {err}"),
            ToolDisplayError::OldTextRequired => "old_text ist erforderlich".to_string(),
            ToolDisplayError::NewTextRequired => "new_text ist erforderlich".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text wurde nicht gefunden".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text wurde {count} Mal gefunden; setze replace_all=true oder gib mehr Kontext an"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Kein Verzeichnis: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Auflisten fehlgeschlagen: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("ungültiger Regex: {err}"),
            ToolDisplayError::CommandTooLong => "Befehl ist zu lang".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "Shell-Metazeichen/Substitutionen sind nicht erlaubt".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Keine Bash-Präfixe erlaubt. Nutze --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' ist nicht in der Bash-Erlaubnisliste")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd ist kein Verzeichnis: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("temporäre stdout-Datei fehlgeschlagen: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("temporäre stderr-Datei fehlgeschlagen: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Ausführung fehlgeschlagen: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Befehl nach {sec}s abgebrochen")
            }
            ToolDisplayError::WaitFailed(err) => format!("Warten fehlgeschlagen: {err}"),
            ToolDisplayError::Required(key) => format!("{key} ist erforderlich"),
            ToolDisplayError::NoAllowedDirs => "keine erlaubten Verzeichnisse konfiguriert".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("Auflösung fehlgeschlagen {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("Pfad existiert nicht: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("übergeordnetes Verzeichnis konnte nicht aufgelöst werden: {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("Pfad liegt außerhalb der erlaubten Verzeichnisse: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("ungültiges Glob-Muster: {err}"),
        },
        Language::Es => match error {
            ToolDisplayError::UnknownTool(name) => format!("Herramienta desconocida '{name}'"),
            ToolDisplayError::NotFile(path) => format!("No es un archivo: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Falló la lectura: {err}"),
            ToolDisplayError::ContentRequired => "content es obligatorio".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content supera {size} bytes")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("No se pudo crear el directorio padre: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Falló la escritura: {err}"),
            ToolDisplayError::OldTextRequired => "old_text es obligatorio".to_string(),
            ToolDisplayError::NewTextRequired => "new_text es obligatorio".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text no se encontró".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text coincidió {count} veces; usa replace_all=true o da más contexto"
            ),
            ToolDisplayError::NotDirectory(path) => format!("No es un directorio: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Falló el listado: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("regex no válida: {err}"),
            ToolDisplayError::CommandTooLong => "comando demasiado largo".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "no se permiten metacaracteres ni sustituciones de shell".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "No hay prefijos Bash permitidos. Usa --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' no está en la lista permitida de Bash")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd no es un directorio: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("falló el stdout temporal: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("falló el stderr temporal: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Falló la ejecución: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("El comando agotó el tiempo tras {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Falló la espera: {err}"),
            ToolDisplayError::Required(key) => format!("{key} es obligatorio"),
            ToolDisplayError::NoAllowedDirs => "no hay directorios permitidos configurados".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("no se pudo resolver {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("la ruta no existe: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("no se pudo resolver el directorio padre de {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("la ruta está fuera de los directorios permitidos: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("patrón glob no válido: {err}"),
        },
        Language::Fr => match error {
            ToolDisplayError::UnknownTool(name) => format!("Outil inconnu '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Ce n'est pas un fichier : {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Échec de lecture : {err}"),
            ToolDisplayError::ContentRequired => "content est requis".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content dépasse {size} octets")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("Échec de création du répertoire parent : {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Échec d'écriture : {err}"),
            ToolDisplayError::OldTextRequired => "old_text est requis".to_string(),
            ToolDisplayError::NewTextRequired => "new_text est requis".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text est introuvable".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text correspond {count} fois ; définissez replace_all=true ou ajoutez du contexte"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Ce n'est pas un répertoire : {path}"),
            ToolDisplayError::ListFailed(err) => format!("Échec du listage : {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("regex invalide : {err}"),
            ToolDisplayError::CommandTooLong => "commande trop longue".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "les métacaractères/substitutions shell ne sont pas autorisés".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Aucun préfixe Bash autorisé. Utilisez --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' n'est pas dans la liste Bash autorisée")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd n'est pas un répertoire : {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("échec du stdout temporaire : {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("échec du stderr temporaire : {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Échec d'exécution : {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("La commande a expiré après {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Échec d'attente : {err}"),
            ToolDisplayError::Required(key) => format!("{key} est requis"),
            ToolDisplayError::NoAllowedDirs => "aucun répertoire autorisé configuré".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("échec de résolution {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("le chemin n'existe pas : {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("échec de résolution du parent pour {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("le chemin est hors des répertoires autorisés : {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("motif glob invalide : {err}"),
        },
        Language::Ja => match error {
            ToolDisplayError::UnknownTool(name) => format!("不明なツール '{name}'"),
            ToolDisplayError::NotFile(path) => format!("ファイルではありません: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("読み取りに失敗しました: {err}"),
            ToolDisplayError::ContentRequired => "content が必要です".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content が {size} バイトを超えています")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("親ディレクトリの作成に失敗しました: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("書き込みに失敗しました: {err}"),
            ToolDisplayError::OldTextRequired => "old_text が必要です".to_string(),
            ToolDisplayError::NewTextRequired => "new_text が必要です".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text が見つかりません".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text が {count} 回一致しました。replace_all=true を設定するか、さらに文脈を指定してください"
            ),
            ToolDisplayError::NotDirectory(path) => format!("ディレクトリではありません: {path}"),
            ToolDisplayError::ListFailed(err) => format!("一覧取得に失敗しました: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("正規表現が無効です: {err}"),
            ToolDisplayError::CommandTooLong => "コマンドが長すぎます".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "shell メタ文字または置換式は使用できません".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "許可された Bash 接頭辞がありません。--bash-prefix を使用してください。".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' は Bash 許可リストにありません")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd はディレクトリではありません: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("一時 stdout の作成に失敗しました: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("一時 stderr の作成に失敗しました: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("実行に失敗しました: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("コマンドが {sec}s 後にタイムアウトしました")
            }
            ToolDisplayError::WaitFailed(err) => format!("待機に失敗しました: {err}"),
            ToolDisplayError::Required(key) => format!("{key} が必要です"),
            ToolDisplayError::NoAllowedDirs => "許可されたディレクトリが設定されていません".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("パスの解決に失敗しました {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("パスが存在しません: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("親ディレクトリの解決に失敗しました: {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("パスは許可されたディレクトリの外です: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("glob パターンが無効です: {err}"),
        },
        Language::Ru => match error {
            ToolDisplayError::UnknownTool(name) => format!("Неизвестный инструмент '{name}'"),
            ToolDisplayError::NotFile(path) => format!("Не файл: {path}"),
            ToolDisplayError::ReadFailed(err) => format!("Ошибка чтения: {err}"),
            ToolDisplayError::ContentRequired => "требуется content".to_string(),
            ToolDisplayError::ContentExceeds(size) => {
                format!("content превышает {size} байт")
            }
            ToolDisplayError::CreateParentFailed(err) => {
                format!("не удалось создать родительский каталог: {err}")
            }
            ToolDisplayError::WriteFailed(err) => format!("Ошибка записи: {err}"),
            ToolDisplayError::OldTextRequired => "требуется old_text".to_string(),
            ToolDisplayError::NewTextRequired => "требуется new_text".to_string(),
            ToolDisplayError::OldTextNotFound => "old_text не найден".to_string(),
            ToolDisplayError::OldTextMatched(count) => format!(
                "old_text найден {count} раз; задайте replace_all=true или добавьте контекст"
            ),
            ToolDisplayError::NotDirectory(path) => format!("Не каталог: {path}"),
            ToolDisplayError::ListFailed(err) => format!("Ошибка списка: {err}"),
            ToolDisplayError::InvalidRegex(err) => format!("недопустимое regex: {err}"),
            ToolDisplayError::CommandTooLong => "команда слишком длинная".to_string(),
            ToolDisplayError::ShellMetacharacters => {
                "метасимволы/подстановки shell не разрешены".to_string()
            }
            ToolDisplayError::NoBashPrefixes => {
                "Нет разрешенных Bash-префиксов. Используйте --bash-prefix.".to_string()
            }
            ToolDisplayError::NotInBashAllowList(head) => {
                format!("'{head}' отсутствует в списке разрешений Bash")
            }
            ToolDisplayError::CwdNotDirectory(path) => {
                format!("cwd не является каталогом: {path}")
            }
            ToolDisplayError::TempStdoutFailed(err) => {
                format!("ошибка временного stdout: {err}")
            }
            ToolDisplayError::TempStderrFailed(err) => {
                format!("ошибка временного stderr: {err}")
            }
            ToolDisplayError::ExecFailed(err) => format!("Ошибка выполнения: {err}"),
            ToolDisplayError::CommandTimedOut(sec) => {
                format!("Команда превысила время ожидания {sec}s")
            }
            ToolDisplayError::WaitFailed(err) => format!("Ошибка ожидания: {err}"),
            ToolDisplayError::Required(key) => format!("требуется {key}"),
            ToolDisplayError::NoAllowedDirs => "не настроены разрешенные каталоги".to_string(),
            ToolDisplayError::FailedResolve(path) => format!("не удалось разрешить {path}"),
            ToolDisplayError::PathDoesNotExist(path) => format!("путь не существует: {path}"),
            ToolDisplayError::FailedResolveParent(path) => {
                format!("не удалось разрешить родительский каталог для {path}")
            }
            ToolDisplayError::OutsideAllowed(path) => {
                format!("путь вне разрешенных каталогов: {path}")
            }
            ToolDisplayError::InvalidGlob(err) => format!("недопустимый glob-шаблон: {err}"),
        },
    }
}

fn quoted_suffix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix(prefix)?.strip_suffix('\'')
}

#[cfg(test)]
mod tests {
    #[test]
    fn permission_profile_copy_does_not_promise_unrestricted_execution() {
        use super::*;
        assert!(text_for(Language::ZhHans, Message::InteractiveAllowPanelAllowAllDesc).contains("系统权限"));
        assert!(!text_for(Language::ZhHans, Message::InteractiveAllowPanelCautiousDesc).contains("仅在"));
        let allow_all = text_for(Language::En, Message::InteractiveAllowPanelAllowAllDesc);
        assert!(allow_all.contains("No Nola approval prompts"));
        assert!(allow_all.contains("System permissions still apply"));
    }
    use super::{
        browser_accept_language_for, browser_locale_for, help_text_for, interaction_text_for,
        language_from_tag, localize_tool_display_name_for, localize_tool_error_display_for,
        text_for, ui_text_for, Language, Message, UiText,
    };

    #[test]
    fn detects_supported_locale_tags() {
        assert_eq!(language_from_tag("zh-Hans"), Language::ZhHans);
        assert_eq!(language_from_tag("zh_CN.UTF-8"), Language::ZhHans);
        assert_eq!(language_from_tag("zh-Hant"), Language::ZhHant);
        assert_eq!(language_from_tag("zh-TW"), Language::ZhHant);
        assert_eq!(language_from_tag("zh_HK.UTF-8"), Language::ZhHant);
        assert_eq!(language_from_tag("en_US.UTF-8"), Language::En);
        assert_eq!(language_from_tag("de_DE.UTF-8"), Language::De);
        assert_eq!(language_from_tag("es-ES"), Language::Es);
        assert_eq!(language_from_tag("fr_FR.UTF-8"), Language::Fr);
        assert_eq!(language_from_tag("ja_JP.UTF-8"), Language::Ja);
        assert_eq!(language_from_tag("ko"), Language::Ko);
        assert_eq!(language_from_tag("ko_KR.UTF-8"), Language::Ko);
        assert_eq!(language_from_tag("ru_RU.UTF-8"), Language::Ru);
    }

    #[test]
    fn set_language_updates_override_and_respects_nola_lang() {
        let _guard = crate::store::test_env_lock();
        let prev_cli_lang = std::env::var_os("NOLA_CLI_LANG");
        let prev_lang = std::env::var_os("NOLA_LANG");
        super::clear_language_override_for_test();
        std::env::remove_var("NOLA_CLI_LANG");
        std::env::set_var("NOLA_LANG", "ko-KR");
        assert_eq!(super::current_language(), Language::Ko);

        super::set_language(Language::En);
        assert_eq!(super::current_language(), Language::En);
        super::set_language(Language::ZhHant);
        assert_eq!(super::current_language(), Language::ZhHant);

        super::clear_language_override_for_test();
        match prev_cli_lang {
            Some(value) => std::env::set_var("NOLA_CLI_LANG", value),
            None => std::env::remove_var("NOLA_CLI_LANG"),
        }
        match prev_lang {
            Some(value) => std::env::set_var("NOLA_LANG", value),
            None => std::env::remove_var("NOLA_LANG"),
        }
    }

    #[test]
    fn falls_back_to_english_for_unknown_locale_tags() {
        assert_eq!(language_from_tag("C"), Language::En);
        assert_eq!(language_from_tag("it_IT.UTF-8"), Language::En);
    }

    #[test]
    fn localizes_common_tool_display_names_without_hiding_unknown_names() {
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "mcp_resources_read"),
            "读取 MCP 资源"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "mcp_resource_templates_read"),
            "Read MCP template resource"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "git_diff"),
            "检查变更"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "project_overview"),
            "查看项目概览"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "codebase_search"),
            "搜索文件"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "codebase_index_status"),
            "Inspect workspace"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "focused_context"),
            "检查工作区"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "find_files"),
            "浏览目录"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "tool_search"),
            "搜索工具"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "computer_click"),
            "操作电脑"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::En, "read_file"),
            "Read file"
        );
        assert_eq!(
            localize_tool_display_name_for(Language::ZhHans, "custom_tool"),
            "工具 · custom_tool"
        );
    }

    #[test]
    fn localizes_interaction_panels_without_cross_language_fallbacks() {
        let english = interaction_text_for(Language::En);
        assert_eq!(english.custom_answer, "Custom answer…");
        assert_eq!(english.resume_title, "Resume session");
        assert!(!english.selection_hint.contains('选'));
        assert!(!english.question_cancelled.contains('取'));

        let chinese = interaction_text_for(Language::ZhHans);
        assert_eq!(chinese.custom_answer, "自定义回答…");
        assert_eq!(chinese.resume_title, "恢复会话");
        assert!(!chinese.selection_hint.contains("confirm"));
        assert!(!chinese.validation_pipeline_rejected.contains("must"));
    }

    #[test]
    fn localizes_secondary_cli_surfaces_for_every_supported_language() {
        let languages = [
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ko,
            Language::Ru,
        ];
        let keys = [
            UiText::Answer,
            UiText::SessionBusy,
            UiText::TasksUsage,
            UiText::HelpAgent,
            UiText::HelpRoom,
            UiText::NoRooms,
            UiText::PermissionInstructionOption,
            UiText::PermissionInstructionCommand,
            UiText::NoConfiguredMcp,
            UiText::NoAvailableSkills,
            UiText::MemoryHeading,
            UiText::MemorySearchHeading,
            UiText::MemorySaved,
            UiText::NoSavedMemory,
            UiText::NoMatchingMemory,
            UiText::NoBackgroundTasks,
            UiText::WaitingPermission,
            UiText::ConfigNotFound,
            UiText::ExecutionPlan,
            UiText::EndpointInvalidScheme,
            UiText::ModelRequired,
        ];
        for language in languages {
            for key in keys {
                assert!(
                    !ui_text_for(language, key).trim().is_empty(),
                    "{language:?} {key:?}"
                );
            }
        }
        assert_eq!(ui_text_for(Language::ZhHans, UiText::Answer), "回答");
        assert_eq!(ui_text_for(Language::ZhHant, UiText::Session), "工作階段");
        assert_eq!(ui_text_for(Language::Ko, UiText::Answer), "답변");
        assert_eq!(
            ui_text_for(Language::Ja, UiText::WaitingPermission),
            "承認待ち"
        );
        assert_eq!(
            ui_text_for(Language::De, UiText::NoAvailableModels),
            "Keine Modelle verfügbar"
        );
        assert!(!ui_text_for(Language::Fr, UiText::HelpTasks).contains('权'));
    }

    #[test]
    fn provides_localized_help_text_for_supported_languages() {
        let cases = [
            (Language::ZhHans, "用法："),
            (Language::ZhHant, "用法："),
            (Language::En, "Usage:"),
            (Language::De, "Verwendung:"),
            (Language::Es, "Uso:"),
            (Language::Fr, "Utilisation :"),
            (Language::Ja, "使い方:"),
            (Language::Ko, "사용법:"),
            (Language::Ru, "Использование:"),
        ];
        for (language, marker) in cases {
            assert!(help_text_for(language).contains(marker));
        }
    }

    #[test]
    fn yes_help_text_accurately_documents_allow_all_and_permission_profile_override() {
        for language in [
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ko,
            Language::Ru,
        ] {
            let line = help_text_for(language)
                .lines()
                .find(|line| line.contains("--yes, -y"))
                .unwrap();
            assert!(line.contains("init/wiki"));
            assert!(line.contains("allow_all"));
            assert!(line.contains("--permission-profile"));
            assert!(!text_for(language, Message::PermissionDeniedGuidance).contains("--yes"));
            assert!(
                !text_for(language, Message::DelegateAgentConfirmationRequired).contains("--yes")
            );
        }
    }

    #[test]
    fn primary_help_does_not_advertise_removed_agent_modes() {
        for language in [Language::ZhHans, Language::ZhHant, Language::En, Language::Ko] {
            let help = help_text_for(language);
            assert!(!help.contains("--swe-mode"));
            assert!(!help.contains("--agent-mode"));
            assert!(help.contains("--resume"));
            assert!(help.contains("--model, -m"));
            assert!(help.contains("NOLA_LANG"));
            assert!(help.contains("NOLA_CLI_DEBUG_LOG_MAX_BYTES"));
        }
    }

    #[test]
    fn provides_browser_locale_for_supported_languages() {
        let cases = [
            (Language::ZhHans, "zh-CN", "zh-CN,zh;q=0.9"),
            (Language::ZhHant, "zh-TW", "zh-TW,zh-Hant;q=0.9"),
            (Language::En, "en-US", "en-US,en;q=0.9"),
            (Language::De, "de-DE", "de-DE,de;q=0.9"),
            (Language::Es, "es-ES", "es-ES,es;q=0.9"),
            (Language::Fr, "fr-FR", "fr-FR,fr;q=0.9"),
            (Language::Ja, "ja-JP", "ja-JP,ja;q=0.9"),
            (Language::Ko, "ko-KR", "ko-KR,ko;q=0.9"),
            (Language::Ru, "ru-RU", "ru-RU,ru;q=0.9"),
        ];
        for (language, locale, accept_language_prefix) in cases {
            assert_eq!(browser_locale_for(language), locale);
            assert!(
                browser_accept_language_for(language).starts_with(accept_language_prefix),
                "{language:?} should prioritize its own browser language"
            );
        }
    }

    #[test]
    fn localizes_interaction_guidance_for_supported_languages() {
        let languages = [
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ko,
            Language::Ru,
        ];
        let messages = [
            Message::PermissionDeniedGuidance,
            Message::SubAgentLabel,
            Message::TaskLabel,
            Message::CapabilityLabel,
            Message::PromptLabel,
            Message::DelegateAgentCapability,
            Message::DelegateAgentConfirmationRequired,
            Message::NetworkAskNonInteractive,
            Message::NetworkAskPrompt,
            Message::NetworkAllowedByUser,
            Message::NetworkDeniedByUser,
        ];
        for language in languages {
            for message in messages {
                let text = text_for(language, message);
                assert!(!text.trim().is_empty(), "{language:?} {message:?}");
            }
        }
        assert_eq!(text_for(Language::ZhHans, Message::TaskLabel), "任务：");
        assert_eq!(text_for(Language::ZhHant, Message::ErrorPrefix), "錯誤：");
        assert_eq!(text_for(Language::Ko, Message::TaskLabel), "작업:");
        assert_eq!(
            text_for(Language::En, Message::NetworkAskPrompt),
            "Allow model network request now? [y/N]: "
        );
    }

    #[test]
    fn localizes_interactive_debug_and_compaction_surface() {
        let messages = [
            Message::InteractiveWelcomeHint,
            Message::InteractiveContextLabel,
            Message::InteractiveSlashCompactHint,
            Message::InteractiveCompactCompleted,
            Message::InteractiveCompactHistoryPreserved,
        ];
        for message in messages {
            assert!(!text_for(Language::En, message).trim().is_empty());
            assert!(!text_for(Language::ZhHans, message).trim().is_empty());
            assert!(!text_for(Language::ZhHant, message).trim().is_empty());
            assert!(!text_for(Language::Ko, message).trim().is_empty());
        }
        assert_eq!(
            text_for(Language::ZhHans, Message::InteractiveContextLabel),
            "上下文"
        );
        assert_eq!(
            text_for(Language::ZhHans, Message::InteractiveCompactCompleted),
            "压缩完成"
        );
    }

    #[test]
    fn every_help_locale_exposes_the_complete_cli_surface() {
        let languages = [
            Language::ZhHans,
            Language::ZhHant,
            Language::En,
            Language::De,
            Language::Es,
            Language::Fr,
            Language::Ja,
            Language::Ko,
            Language::Ru,
        ];
        let required_commands = [
            "nola init",
            "nola wiki",
            "nola web",
            "nola skills",
            "nola mcp",
            "nola memory",
            "nola model",
            "nola update",
            "nola version",
            "nola sessions",
            "nola session show",
            "nola session delete",
        ];
        let required_options = [
            "--endpoint",
            "--model, -m",
            "--project",
            "--debug",
            "--permission-profile",
            "--yes, -y",
            "--network-policy",
            "--force-tool-set",
            "--resume",
            "--output",
            "--dry-run",
            "--host",
            "--port",
            "--web-root",
            "--version, -V",
        ];
        let required_environment = [
            "NOLA_CLI_LANG",
            "NOLA_LANG",
            "NOLA_PERMISSION_PROFILE",
            "NOLA_AGENT_CONTEXT_WINDOW_TOKENS",
            "NOLA_CLI_DEBUG_LOG_PATH",
            "NOLA_CLI_INTERACTION_MODE",
            "NOLA_DB_PATH",
        ];

        for language in languages {
            let help = super::help_text_for(language);
            for needle in required_commands
                .iter()
                .chain(required_options.iter())
                .chain(required_environment.iter())
            {
                assert!(
                    help.contains(needle),
                    "{language:?} help is missing {needle}"
                );
            }
        }
    }

    #[test]
    fn localizes_common_tool_error_display_text() {
        assert_eq!(
            localize_tool_error_display_for(Language::ZhHans, "Not a file: README.md"),
            "不是文件：README.md"
        );
        assert_eq!(
            localize_tool_error_display_for(
                Language::De,
                "path is outside allowed directories: /tmp/x"
            ),
            "Pfad liegt außerhalb der erlaubten Verzeichnisse: /tmp/x"
        );
        assert_eq!(
            localize_tool_error_display_for(Language::Ja, "old_text was not found"),
            "old_text が見つかりません"
        );
    }
}
