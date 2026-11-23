// SPDX-License-Identifier: Apache-2.0
// Explicit room syntax only; natural-language content never selects recipients.
fn room_text(values: [&'static str; 9]) -> &'static str {
    crate::i18n::pick_language(crate::i18n::current_language(), values)
}

fn interactive_room_snapshot(id: &str) -> Result<nola_application::agent_room::RoomSnapshot,String> {
    let value: Option<nola_application::agent_room::RoomSnapshot> = application_port::CliApplicationPort::persistent_default()?
        .execute(nola_application::ApplicationCommand::AgentRoom { command:nola_application::agent_room::AgentRoomCommand::Read {room_id:id.into()} })?;
    value.ok_or_else(|| {
        room_text([
            "未找到房间",
            "找不到房間",
            "Room not found",
            "Raum nicht gefunden",
            "Sala no encontrada",
            "Salon introuvable",
            "ルームが見つかりません",
            "방을 찾을 수 없습니다",
            "Комната не найдена",
        ])
        .into()
    })
}

fn room_mention_token(member: &nola_domain::agent_room::RoomParticipant, members: &[nola_domain::agent_room::RoomParticipant]) -> String {
    // Duplicate or whitespace-containing names use the stable candidate ID.
    // Tab still presents the public name, and submit resolves only exact tokens.
    if !member.display_name.chars().any(char::is_whitespace) && !member.display_name.starts_with('@')
        && members.iter().filter(|other|other.active && other.display_name == member.display_name).count() == 1 {
        member.display_name.clone()
    } else { member.id.clone() }
}

fn room_explicit_recipient<'a>(text: &'a str, members: &[nola_domain::agent_room::RoomParticipant]) -> Result<(Option<String>, &'a str),String> {
    let Some(mention) = text.strip_prefix('@') else { return Ok((None,text)); };
    let (token,body) = mention.split_once(char::is_whitespace).ok_or_else(|| {
        room_text([
            "请先用 Tab 选择 @接收者，再输入消息",
            "請先用 Tab 選擇 @接收者，再輸入訊息",
            "Choose an @recipient with Tab, then enter a message",
            "Wählen Sie einen @Empfänger mit Tab und geben Sie dann eine Nachricht ein",
            "Elige un @destinatario con Tab y luego escribe un mensaje",
            "Choisissez un @destinataire avec Tab, puis saisissez un message",
            "Tab で @宛先 を選択してからメッセージを入力してください",
            "Tab으로 @수신자를 선택한 뒤 메시지를 입력하세요",
            "Выберите @получателя клавишей Tab, затем введите сообщение",
        ])
    })?;
    let mut matching = members.iter().filter(|member|member.active && room_mention_token(member,members) == token);
    let member = matching.next()
        .ok_or_else(|| {
            room_text([
                "未知或不明确的 @接收者；请使用房间候选列表",
                "未知或不明確的 @接收者；請使用房間候選清單",
                "Unknown or ambiguous @recipient; use the room candidates",
                "Unbekannter oder mehrdeutiger @Empfänger; verwenden Sie die Raumkandidaten",
                "@destinatario desconocido o ambiguo; usa los candidatos de la sala",
                "@destinataire inconnu ou ambigu ; utilisez les candidats du salon",
                "不明または曖昧な @宛先 です。ルームの候補を使用してください",
                "알 수 없거나 모호한 @수신자입니다. 방 후보 목록을 사용하세요",
                "Неизвестный или неоднозначный @получатель; используйте список участников комнаты",
            ])
        })?;
    if matching.next().is_some() {
        return Err(room_text([
            "@接收者不明确；请使用唯一的房间候选标识",
            "@接收者不明確；請使用唯一的房間候選識別碼",
            "Ambiguous @recipient; use a unique room candidate",
            "Mehrdeutiger @Empfänger; verwenden Sie einen eindeutigen Raumkandidaten",
            "@destinatario ambiguo; usa un candidato único de la sala",
            "@destinataire ambigu ; utilisez un candidat unique du salon",
            "@宛先 が曖昧です。一意のルーム候補を使用してください",
            "@수신자가 모호합니다. 고유한 방 후보를 사용하세요",
            "Неоднозначный @получатель; укажите уникального участника комнаты",
        ]).into());
    }
    let body=body.trim_start();
    if body.is_empty() || body.starts_with('@') {
        return Err(room_text([
            "请选择一位接收者并输入消息",
            "請選擇一位接收者並輸入訊息",
            "Choose one recipient and enter a message",
            "Wählen Sie einen Empfänger und geben Sie eine Nachricht ein",
            "Elige un destinatario y escribe un mensaje",
            "Choisissez un destinataire et saisissez un message",
            "宛先を1つ選択してメッセージを入力してください",
            "수신자 한 명을 선택하고 메시지를 입력하세요",
            "Выберите одного получателя и введите сообщение",
        ]).into());
    }
    Ok((Some(member.id.clone()),body))
}

fn interactive_room_completions(rest: &str) -> Vec<SlashCompletion> {
    let Some((id,tail)) = rest.split_once(char::is_whitespace) else { return vec![]; };
    let Some(query)=tail.trim_start().strip_prefix('@') else { return vec![]; };
    if query.chars().any(char::is_whitespace) { return vec![]; }
    let Ok(room)=interactive_room_snapshot(id) else { return vec![]; };
    room.room.participants().iter().filter(|member|member.active)
        .filter(|member|member.display_name.to_lowercase().starts_with(&query.to_lowercase()) || member.id.starts_with(query))
        .map(|member| {
            let token = room_mention_token(member, room.room.participants());
            let label = if token == member.display_name {
                format!("@{}", member.display_name)
            } else {
                format!("@{} · {token}", member.display_name)
            };
            SlashCompletion { name: format!("/room {id} @{token} "), label: Some(label) }
        })
        .take(4).collect()
}

fn interactive_room_command(rest: &str, config: &Config) -> Result<(),String> {
    use nola_application::{ApplicationCommand,agent_room::{AgentRoomCommand,RoomSnapshot,RoomSubmission,SubmitRoomMessage}};
    let mut app=application_port::CliApplicationPort::persistent_default()?;
    if rest.is_empty() {
        let rooms:Vec<RoomSnapshot>=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::List {limit:100,offset:0}})?;
        if rooms.is_empty() {
            terminal_notice!("  {}", ui_text(UiText::NoRooms));
            terminal_notice!(r#"  nola room create "Team""#);
            return Ok(());
        }
        for room in rooms { terminal_notice!("  {}  {}",room.room.id(),room.room.title()); }
        terminal_notice!("  /room <room-id> [@recipient] <message>");
        terminal_notice!("  {}", ui_text(UiText::HelpRoom));
        return Ok(());
    }
    let (id,text)=rest.split_once(char::is_whitespace).unwrap_or((rest,""));
    let room=interactive_room_snapshot(id)?;
    if text.trim().is_empty() {
        terminal_notice!("  {}",room.room.title());
        for member in room.room.participants().iter().filter(|member|member.active) {
            let token = room_mention_token(member,room.room.participants());
            if token == member.display_name { terminal_notice!("  @{}", member.display_name); }
            else { terminal_notice!("  @{} — {}", token, member.display_name); }
        }
        terminal_notice!("  /room {id} [@recipient] <message>");
        terminal_notice!("  {}", ui_text(UiText::HelpRoom));
        return Ok(());
    }
    let (participant_id,text)=room_explicit_recipient(text.trim_start(),room.room.participants())?;
    let submission_id=uuid::Uuid::new_v4().to_string();
    let sending_notice = match crate::i18n::current_language() {
        crate::i18n::Language::ZhHans => format!("正在发送到房间 {}…", room.room.title()),
        crate::i18n::Language::ZhHant => format!("正在傳送至房間 {}…", room.room.title()),
        crate::i18n::Language::De => format!("Sende an Raum {}…", room.room.title()),
        crate::i18n::Language::Es => format!("Enviando a la sala {}…", room.room.title()),
        crate::i18n::Language::Fr => format!("Envoi au salon {}…", room.room.title()),
        crate::i18n::Language::Ja => format!("ルーム {} に送信中…", room.room.title()),
        crate::i18n::Language::Ko => format!("방 {}에 전송 중…", room.room.title()),
        crate::i18n::Language::Ru => format!("Отправка в комнату {}…", room.room.title()),
        crate::i18n::Language::En => format!("Sending to room {}…", room.room.title()),
    };
    terminal_notice!("  {sending_notice}");
    let accepted:RoomSubmission=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::Submit {input:SubmitRoomMessage {
        room_id:id.into(),submission_id:submission_id.clone(),expected_room_revision:room.revision,text:text.into(),participant_id,at_ms:chrono::Utc::now().timestamp_millis(),parent_message_id:None,conversation_id:None,
    }}})?;
    terminal_notice!(
        "  {}",
        room_text([
            "已受理 · 处理中…",
            "已受理 · 處理中…",
            "Accepted · processing…",
            "Angenommen · wird verarbeitet…",
            "Aceptado · procesando…",
            "Accepté · traitement en cours…",
            "受付済み · 処理中…",
            "접수됨 · 처리 중…",
            "Принято · обработка…",
        ])
    );
    let cancellation=interaction::ExecutionCancellation::interactive();
    let _scope=cancellation.enter();
    let _signals=crate::execution_signals::ExecutionSignals::install(cancellation.runtime_control(),None)?;
    let listener=interaction::start_execution_cancel_listener(&cancellation);
    let execution=crate::web::run_room_submission_with_permission(config,accepted,Some(interactive_room_permission));
    drop(listener);
    cancellation.wait_for_terminal_listener_release();
    execution?;
    let intent:Option<nola_application::agent_room::RoomExecutionIntent>=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ExecutionIntent {room_id:id.into(),submission_id:submission_id.clone()}})?;
    let value:RoomSubmission=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ReconcileExecution {room_id:id.into(),submission_id,at_ms:chrono::Utc::now().timestamp_millis()}})?;
    let recipient=room.room.participants().iter().find(|member|Some(&member.id)==value.selected_participant_id.as_ref());
    terminal_notice!("  @{} · {:?}",recipient.map(|member|member.display_name.as_str()).unwrap_or("—"),value.state);
    if let Some(context)=value.context_conversation_id {
        if let Some(intent)=intent {
            let assistant=format!("{}:assistant",intent.execution_id);
            for message in session::load_session_messages_with_ids(&context)? {
                if message["id"].as_str()==Some(assistant.as_str()) {
                    if let Some(text)=message["content"].as_str() { terminal_notice!("{}",text); }
                }
            }
        }
        terminal_notice!(
            "  /resume {context}  — {}",
            room_text([
                "查看完整上下文与待处理权限",
                "檢視完整上下文與待處理權限",
                "full context and pending permissions",
                "vollständiger Kontext und ausstehende Berechtigungen",
                "contexto completo y permisos pendientes",
                "contexte complet et autorisations en attente",
                "完全なコンテキストと保留中の権限",
                "전체 컨텍스트 및 대기 중인 권한",
                "полный контекст и ожидающие разрешения",
            ])
        );
    }
    Ok(())
}

fn interactive_room_permission(event: &serde_json::Value) -> Result<serde_json::Value,String> {
    use crate::runtime_interaction::{PermissionPrompt,PermissionPromptKind};
    let challenge: nola_core::host_auth::HostPermissionChallenge = serde_json::from_value(event.clone())
        .map_err(|_| {
            room_text([
                "无效的房间权限请求",
                "無效的房間權限請求",
                "Invalid room permission challenge",
                "Ungültige Raum-Berechtigungsanfrage",
                "Solicitud de permiso de sala no válida",
                "Demande d’autorisation de salon invalide",
                "無効なルーム権限リクエストです",
                "유효하지 않은 방 권한 요청입니다",
                "Недопустимый запрос разрешения комнаты",
            ])
        })?;
    if challenge.protocol_version != nola_core::host_auth::HOST_PERMISSION_PROTOCOL_VERSION
        || challenge.expires_at <= u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0) {
        return Err(room_text([
            "房间权限请求已过期或不受支持",
            "房間權限請求已過期或不受支援",
            "Room permission challenge is expired or unsupported",
            "Raum-Berechtigungsanfrage ist abgelaufen oder wird nicht unterstützt",
            "La solicitud de permiso de sala expiró o no es compatible",
            "La demande d’autorisation de salon a expiré ou n’est pas prise en charge",
            "ルーム権限リクエストは期限切れか未対応です",
            "방 권한 요청이 만료되었거나 지원되지 않습니다",
            "Запрос разрешения комнаты истёк или не поддерживается",
        ]).into());
    }
    let review = event["safe_review_text"].as_str().filter(|text|!text.trim().is_empty())
        .ok_or_else(|| {
            room_text([
                "该房间操作缺少可安全审查的详情；请打开其上下文查看",
                "該房間操作缺少可安全審查的詳情；請開啟其上下文檢視",
                "Room operation has no safe review details; open its context to inspect",
                "Raumvorgang hat keine sicheren Prüfdetails; öffnen Sie den Kontext zur Prüfung",
                "La operación de la sala no tiene detalles seguros para revisar; abre su contexto para inspeccionar",
                "L’opération du salon ne contient pas de détails vérifiables ; ouvrez son contexte pour l’inspecter",
                "このルーム操作には安全な確認詳細がありません。コンテキストを開いて確認してください",
                "방 작업에 안전하게 검토할 세부 정보가 없습니다. 컨텍스트를 열어 확인하세요",
                "У операции комнаты нет безопасных сведений для проверки; откройте её контекст",
            ])
        })?;
    let room_label = room_text([
        "房间",
        "房間",
        "Room",
        "Raum",
        "Sala",
        "Salon",
        "ルーム",
        "방",
        "Комната",
    ]);
    let awaiting_label = room_text([
        "等待权限",
        "等待權限",
        "awaiting permission",
        "wartet auf Berechtigung",
        "esperando permiso",
        "en attente d’autorisation",
        "許可待ち",
        "권한 대기 중",
        "ожидание разрешения",
    ]);
    let context_label = room_text([
        "上下文",
        "上下文",
        "Context",
        "Kontext",
        "Contexto",
        "Contexte",
        "コンテキスト",
        "컨텍스트",
        "Контекст",
    ]);
    let operation_label = room_text([
        "操作",
        "操作",
        "Operation",
        "Vorgang",
        "Operación",
        "Opération",
        "操作",
        "작업",
        "Операция",
    ]);
    let allow_context_label = room_text([
        "本上下文允许",
        "本上下文允許",
        "Allow for this context",
        "Für diesen Kontext erlauben",
        "Permitir para este contexto",
        "Autoriser pour ce contexte",
        "このコンテキストで許可",
        "이 컨텍스트에서 허용",
        "Разрешить для этого контекста",
    ]);
    let instruction_label = room_text([
        "提供指令",
        "提供指令",
        "Provide instruction",
        "Anweisung geben",
        "Dar instrucción",
        "Fournir une instruction",
        "指示を入力",
        "지시 입력",
        "Дать указание",
    ]);
    let prompt = PermissionPrompt {
        allow_conversation:challenge.allow_conversation, kind:PermissionPromptKind::Tool,
        title:format!("{room_label} · {} · {awaiting_label}",challenge.tool_name),
        rows:vec![(context_label.into(),challenge.session_id.clone()),(operation_label.into(),review.into())],
        session_label:allow_context_label.into(),session_value:challenge.session_id.clone(),
        custom_label:instruction_label.into(),supports_instruction:true,
    };
    let resolution = room_permission_resolution(interaction::prompt_permission(&prompt))?;
    Ok(serde_json::json!({"protocol_version":challenge.protocol_version,"nonce":challenge.nonce,
        "request_id":challenge.request_id,"host_id":challenge.host_id,"client_id":challenge.client_id,
        "session_id":challenge.session_id,"tool_name":challenge.tool_name,"resolution":resolution}))
}

fn room_permission_resolution(choice: crate::runtime_interaction::PermissionChoice) -> Result<serde_json::Value,String> {
    use crate::runtime_interaction::PermissionChoice;
    Ok(match choice {
        PermissionChoice::Once => serde_json::json!({"type":"authorization","decision":"allow_once"}),
        PermissionChoice::Session(_) => serde_json::json!({"type":"authorization","decision":"allow_conversation"}),
        PermissionChoice::Deny => serde_json::json!({"type":"authorization","decision":"deny"}),
        PermissionChoice::ReturnToComposer => serde_json::json!({"type":"intervention","intervention":{"type":"return_to_composer"}}),
        PermissionChoice::Custom(instruction) => serde_json::json!({"type":"intervention","intervention":{"type":"provide_instruction","instruction":instruction}}),
        PermissionChoice::TransportInterrupted(_) => return Err(room_text([
            "房间权限输入已中断；继续前请检查上下文",
            "房間權限輸入已中斷；繼續前請檢查上下文",
            "Room permission input interrupted; inspect the context before continuing",
            "Eingabe für Raumberechtigung unterbrochen; prüfen Sie den Kontext, bevor Sie fortfahren",
            "Entrada de permiso de sala interrumpida; inspecciona el contexto antes de continuar",
            "Saisie d’autorisation du salon interrompue ; inspectez le contexte avant de continuer",
            "ルーム権限の入力が中断されました。続行前にコンテキストを確認してください",
            "방 권한 입력이 중단되었습니다. 계속하기 전에 컨텍스트를 확인하세요",
            "Ввод разрешения комнаты прерван; проверьте контекст перед продолжением",
        ]).into()),
    })
}

#[cfg(test)]
mod interactive_room_mentions_tests {
    use super::*;
    #[test]
    fn room_permission_choices_use_canonical_resolution_without_implicit_approval() {
        use crate::runtime_interaction::PermissionChoice;
        for choice in [PermissionChoice::Once,PermissionChoice::Session("context".into()),PermissionChoice::Deny,
            PermissionChoice::ReturnToComposer,PermissionChoice::Custom("do not write".into())] {
            let resolution=room_permission_resolution(choice).unwrap();
            assert!(serde_json::from_value::<nola_host_protocol::HostPermissionResolution>(resolution).is_ok());
        }
        assert_eq!(room_permission_resolution(PermissionChoice::ReturnToComposer).unwrap()["type"],"intervention");
        assert_eq!(room_permission_resolution(PermissionChoice::Deny).unwrap()["decision"],"deny");
        assert!(room_permission_resolution(PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Disconnected)).is_err());
    }
    fn member(id:&str,name:&str)->nola_domain::agent_room::RoomParticipant {
        nola_domain::agent_room::RoomParticipant {id:id.into(),agent_id:nola_domain::persistent_agent::AgentId::try_from(id.to_owned()).unwrap(),agent_revision:1,display_name:name.into(),description:String::new(),context_conversation_id:format!("context-{id}"),active:true}
    }
    #[test]
    fn only_explicit_leading_candidate_changes_recipient() {
        let members=vec![member("one","Research"),member("two","写作")];
        for text in ["search 当前 follow-up","email a@b.com","translate @Research literally"] {
            assert_eq!(room_explicit_recipient(text,&members).unwrap(),(None,text));
        }
        assert_eq!(room_explicit_recipient("@写作 你好",&members).unwrap(),(Some("two".into()),"你好"));
        assert!(room_explicit_recipient("@unknown hello",&members).is_err());
        assert!(room_explicit_recipient("@Research @写作 hello",&members).is_err());
    }
    #[test]
    fn duplicate_names_require_exact_candidate_identity() {
        let members=vec![member("one","Same"),member("two","Same")];
        assert!(room_explicit_recipient("@Same hello",&members).is_err());
        assert_eq!(room_explicit_recipient("@two hello",&members).unwrap().0,Some("two".into()));
    }
}
