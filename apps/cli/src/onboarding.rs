use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use crate::config::Config;
use crate::i18n::{current_language, Language};

#[derive(Clone, Copy)]
struct GuideCopy {
    title: &'static str,
    subtitle: &'static str,
    value_title: &'static str,
    value_body: &'static str,
    runtime_title: &'static str,
    runtime_body: &'static str,
    sources_title: &'static str,
    sources_body: &'static str,
    ready_title: &'static str,
    ready_body: &'static str,
    current_model: &'static str,
    workspace: &'static str,
    not_configured: &'static str,
    continue_hint: &'static str,
    replay_hint: &'static str,
}

pub fn run_if_needed(config: &Config) -> Result<(), String> {
    if skip_requested() || completion_path().is_some_and(|path| path.exists()) {
        return Ok(());
    }

    show(config);
    print!("  {} ", copy().continue_hint);
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| error.to_string())?;
    mark_completed()
}

pub fn show(config: &Config) {
    let copy = copy();
    let model = if config.model.trim().is_empty() {
        copy.not_configured
    } else {
        config.model.trim()
    };
    let workspace = config.project_root.display();
    let use_color = onboarding_color_enabled(io::stdout().is_terminal());

    println!();
    if use_color {
        println!("  \x1b[1;38;5;117mNola\x1b[0m  {}", copy.title);
    } else {
        println!("  Nola  {}", copy.title);
    }
    println!("  {}", copy.subtitle);
    println!();
    print_step(1, copy.value_title, copy.value_body, use_color);
    print_step(2, copy.runtime_title, copy.runtime_body, use_color);
    println!("       {}: {}", copy.current_model, model);
    print_step(3, copy.sources_title, copy.sources_body, use_color);
    println!("       {}: {}", copy.workspace, workspace);
    print_step(4, copy.ready_title, copy.ready_body, use_color);
    println!();
    println!("  {}", copy.replay_hint);
    println!();
}

fn print_step(number: usize, title: &str, body: &str, use_color: bool) {
    if use_color {
        println!("  \x1b[38;5;117m{}\x1b[0m  \x1b[1m{}\x1b[0m", number, title);
    } else {
        println!("  {}  {}", number, title);
    }
    println!("     {}", body);
}

fn onboarding_color_enabled(stdout_is_terminal: bool) -> bool {
    if !stdout_is_terminal {
        return false;
    }
    if env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    if env::var("TERM")
        .ok()
        .is_some_and(|term| term.trim().eq_ignore_ascii_case("dumb"))
    {
        return false;
    }
    true
}

fn skip_requested() -> bool {
    env::var("NOLA_SKIP_ONBOARDING")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes"))
}

fn completion_path() -> Option<PathBuf> {
    crate::platform_paths::home_dir().map(|home| home.join(".nola").join("onboarding-v2.json"))
}

pub(crate) fn mark_completed() -> Result<(), String> {
    let Some(path) = completion_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, "{\"version\":2}\n").map_err(|error| error.to_string())
}

fn copy() -> GuideCopy {
    copy_for_language(current_language())
}

fn copy_for_language(language: Language) -> GuideCopy {
    match language {
        Language::ZhHans => GuideCopy {
            title: "首次使用引导",
            subtitle: "四步了解 Nola，不会在这里集中索取系统权限。",
            value_title: "理解工作方式",
            value_body: "Nola 连接你选择的上下文，展示用过的来源，并让长期记忆保持可编辑。",
            runtime_title: "确认运行方式",
            runtime_body: "CLI 在本机编排工具；模型可以来自当前配置的本地或远程提供方。",
            sources_title: "从工作区开始",
            sources_body: "文件和网络只在任务需要时访问，敏感操作仍会单独请求确认。",
            ready_title: "完成第一个任务",
            ready_body: "直接描述目标，例如“阅读这个项目并列出三个最高风险问题”。",
            current_model: "当前模型",
            workspace: "工作区",
            not_configured: "未配置；运行 nola model manage 查看配置入口和 Host 启动命令",
            continue_hint: "按 Enter 进入 Nola",
            replay_hint: "随时输入 /guide 重新查看本引导。",
        },
        Language::ZhHant => GuideCopy {
            title: "首次使用導覽",
            subtitle: "四個步驟認識 Nola，系統權限不會在這裡一次索取。",
            value_title: "了解運作方式",
            value_body: "Nola 連接您選擇的內容，顯示採用的來源，並讓長期記憶維持可編輯。",
            runtime_title: "確認執行方式",
            runtime_body: "CLI 在本機編排工具；模型來自您設定的本機或遠端提供者。",
            sources_title: "從工作區開始",
            sources_body: "檔案與網路僅在任務需要時存取，敏感操作仍會個別徵詢確認。",
            ready_title: "執行第一個任務",
            ready_body: "直接說明目標，例如：「檢視這個專案並列出三個最高風險的問題」。",
            current_model: "目前模型",
            workspace: "工作區",
            not_configured: "尚未設定；執行 nola model manage 查看設定與 Host 啟動命令",
            continue_hint: "按 Enter 進入 Nola",
            replay_hint: "隨時輸入 /guide 即可再次開啟本導覽。",
        },
        Language::De => GuideCopy {
            title: "Erste Schritte",
            subtitle: "Nola in vier Schritten. Systemrechte werden hier nicht gesammelt angefordert.",
            value_title: "Arbeitsweise verstehen",
            value_body: "Nola verbindet gewählten Kontext, zeigt verwendete Quellen und hält Langzeitwissen bearbeitbar.",
            runtime_title: "Ausführung prüfen",
            runtime_body: "Die CLI koordiniert Werkzeuge lokal; das Modell kommt aus der konfigurierten lokalen oder entfernten Quelle.",
            sources_title: "Mit dem Arbeitsbereich beginnen",
            sources_body: "Dateien und Netzwerk werden erst bei Bedarf verwendet; sensible Aktionen verlangen eine eigene Bestätigung.",
            ready_title: "Erste Aufgabe starten",
            ready_body: "Beschreibe ein Ziel, etwa: „Prüfe dieses Projekt und nenne die drei größten Risiken.“",
            current_model: "Aktuelles Modell",
            workspace: "Arbeitsbereich",
            not_configured: "Nicht konfiguriert; nola model manage zeigt Einrichtung und Host-Start",
            continue_hint: "Enter drücken, um Nola zu öffnen",
            replay_hint: "Mit /guide kannst du diese Einführung erneut öffnen.",
        },
        Language::Es => GuideCopy {
            title: "Primeros pasos",
            subtitle: "Conoce Nola en cuatro pasos. Aquí no se solicitan permisos del sistema en bloque.",
            value_title: "Entiende el flujo",
            value_body: "Nola conecta el contexto elegido, muestra las fuentes usadas y mantiene editable la memoria duradera.",
            runtime_title: "Revisa la ejecución",
            runtime_body: "La CLI coordina herramientas en tu equipo; el modelo usa el proveedor local o remoto configurado.",
            sources_title: "Empieza con el espacio de trabajo",
            sources_body: "Los archivos y la red se usan solo cuando la tarea lo necesita; las acciones sensibles se confirman aparte.",
            ready_title: "Haz la primera tarea",
            ready_body: "Describe un objetivo, por ejemplo: «Revisa este proyecto y enumera los tres riesgos principales».",
            current_model: "Modelo actual",
            workspace: "Espacio de trabajo",
            not_configured: "Sin configurar; nola model manage muestra cómo configurar e iniciar el Host",
            continue_hint: "Pulsa Enter para abrir Nola",
            replay_hint: "Escribe /guide para volver a ver esta guía.",
        },
        Language::Fr => GuideCopy {
            title: "Premiers pas",
            subtitle: "Découvrez Nola en quatre étapes. Aucun lot d’autorisations système n’est demandé ici.",
            value_title: "Comprendre le fonctionnement",
            value_body: "Nola relie le contexte choisi, affiche les sources utilisées et garde la mémoire durable modifiable.",
            runtime_title: "Vérifier l’exécution",
            runtime_body: "La CLI orchestre les outils localement ; le modèle utilise le fournisseur local ou distant configuré.",
            sources_title: "Commencer par l’espace de travail",
            sources_body: "Fichiers et réseau ne sont utilisés qu’au besoin ; les actions sensibles demandent une confirmation séparée.",
            ready_title: "Lancer la première tâche",
            ready_body: "Décrivez un objectif, par exemple : « Analyse ce projet et liste les trois principaux risques ».",
            current_model: "Modèle actuel",
            workspace: "Espace de travail",
            not_configured: "Non configuré ; nola model manage affiche la configuration et le démarrage du Host",
            continue_hint: "Appuyez sur Entrée pour ouvrir Nola",
            replay_hint: "Entrez /guide à tout moment pour revoir ce guide.",
        },
        Language::Ja => GuideCopy {
            title: "はじめてのガイド",
            subtitle: "4 ステップで Nola を理解します。ここで権限をまとめて要求することはありません。",
            value_title: "仕組みを理解する",
            value_body: "選んだコンテキストを接続し、使用した情報源を示し、長期記憶をいつでも編集できます。",
            runtime_title: "実行方法を確認する",
            runtime_body: "CLI はこの端末でツールを調整し、モデルは設定済みのローカルまたはリモートプロバイダーを使います。",
            sources_title: "ワークスペースから始める",
            sources_body: "ファイルとネットワークは必要な時だけ使い、機密操作は個別に確認します。",
            ready_title: "最初のタスクを始める",
            ready_body: "例：「このプロジェクトを確認し、最大のリスクを 3 つ挙げて」のように目的を伝えてください。",
            current_model: "現在のモデル",
            workspace: "ワークスペース",
            not_configured: "未設定。nola model manage で設定先と Host の起動方法を確認",
            continue_hint: "Enter で Nola を開く",
            replay_hint: "/guide でいつでもこのガイドを再表示できます。",
        },
        Language::Ko => GuideCopy {
            title: "처음 사용 가이드",
            subtitle: "4단계로 Nola를 살펴보세요. 여기서는 시스템 권한을 한꺼번에 요청하지 않습니다.",
            value_title: "동작 방식 이해하기",
            value_body: "Nola는 선택한 컨텍스트를 연결하고, 사용한 출처를 표시하며, 장기 기억을 언제든 편집할 수 있게 유지합니다.",
            runtime_title: "실행 방식 확인하기",
            runtime_body: "CLI가 이 기기에서 도구를 조율하며, 모델은 설정된 로컬 또는 원격 공급자를 사용합니다.",
            sources_title: "작업 공간에서 시작하기",
            sources_body: "파일과 네트워크는 작업에 필요할 때만 사용되며, 민감한 작업은 별도로 확인합니다.",
            ready_title: "첫 작업 실행하기",
            ready_body: "예를 들어 “이 프로젝트를 검토하고 가장 위험한 문제 세 가지를 알려줘”처럼 목표를 입력하세요.",
            current_model: "현재 모델",
            workspace: "작업 공간",
            not_configured: "설정되지 않음; nola model manage 에서 설정 및 Host 시작 명령을 확인하세요",
            continue_hint: "Enter를 눌러 Nola 시작",
            replay_hint: "언제든 /guide 를 입력해 이 가이드를 다시 볼 수 있습니다.",
        },
        Language::Ru => GuideCopy {
            title: "Первое знакомство",
            subtitle: "Четыре шага о Nola. Здесь системные разрешения не запрашиваются пакетом.",
            value_title: "Понять принцип работы",
            value_body: "Nola подключает выбранный контекст, показывает использованные источники и оставляет долгую память редактируемой.",
            runtime_title: "Проверить выполнение",
            runtime_body: "CLI управляет инструментами локально, а модель берётся из настроенного локального или удалённого провайдера.",
            sources_title: "Начать с рабочей папки",
            sources_body: "Файлы и сеть используются только по необходимости; чувствительные действия подтверждаются отдельно.",
            ready_title: "Запустить первую задачу",
            ready_body: "Опишите цель, например: «Проверь этот проект и назови три главных риска».",
            current_model: "Текущая модель",
            workspace: "Рабочая папка",
            not_configured: "Не настроено; nola model manage покажет настройки и команду запуска Host",
            continue_hint: "Нажмите Enter, чтобы открыть Nola",
            replay_hint: "Введите /guide, чтобы снова открыть это руководство.",
        },
        Language::En => GuideCopy {
            title: "First-run guide",
            subtitle: "Understand Nola in four steps. System permissions are never requested in bulk here.",
            value_title: "Understand the loop",
            value_body: "Nola connects the context you choose, shows the sources it used, and keeps durable memory editable.",
            runtime_title: "Confirm execution",
            runtime_body: "The CLI orchestrates tools on this machine; the model uses your configured local or remote provider.",
            sources_title: "Start with the workspace",
            sources_body: "Files and network are accessed only when a task needs them; sensitive actions still ask separately.",
            ready_title: "Run the first task",
            ready_body: "State a goal, such as “Review this project and list the three highest-risk issues.”",
            current_model: "Current model",
            workspace: "Workspace",
            not_configured: "Not configured; nola model manage shows setup and the Host start command",
            continue_hint: "Press Enter to enter Nola",
            replay_hint: "Type /guide at any time to replay this guide.",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_hint_points_to_model_management_instead_of_document_generation() {
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
            let hint = copy_for_language(language).not_configured;
            assert!(hint.contains("nola model manage"));
            assert!(!hint.contains("nola init"));
        }
    }

    #[test]
    fn color_output_respects_tty_no_color_and_dumb_term() {
        let _guard = crate::store::test_env_lock();
        let prev_no_color = env::var_os("NO_COLOR");
        let prev_term = env::var_os("TERM");

        env::remove_var("NO_COLOR");
        env::set_var("TERM", "xterm-256color");
        assert!(!onboarding_color_enabled(false));
        assert!(onboarding_color_enabled(true));

        env::set_var("NO_COLOR", "1");
        assert!(!onboarding_color_enabled(true));

        env::remove_var("NO_COLOR");
        env::set_var("TERM", "dumb");
        assert!(!onboarding_color_enabled(true));

        match prev_no_color {
            Some(value) => env::set_var("NO_COLOR", value),
            None => env::remove_var("NO_COLOR"),
        }
        match prev_term {
            Some(value) => env::set_var("TERM", value),
            None => env::remove_var("TERM"),
        }
    }

    #[test]
    fn completion_marker_is_versioned() {
        assert!(completion_path()
            .map(|path| path.ends_with("onboarding-v2.json"))
            .unwrap_or(true));
    }
}
