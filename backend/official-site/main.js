// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements main logic in the backend infrastructure.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
(function () {
    const EN = {
        title: "Nola · Open-source assistant",
        meta_description: "Nola is an open-source personal assistant: custom LLM endpoint and key, local-first data, multilingual and cross-platform.",
        og_description: "Open-source, customizable, privacy-first. Make AI part of your daily workflow.",
        nav_overview: "Overview",
        nav_features: "Features",
        nav_flow: "Flow",
        nav_platform: "Platform",
        nav_faq: "FAQ",
        nav_contact_chip: "Try Nola →",
        nav_contact: "Contact",
        lang_label: "Language",
        hero_eyebrow: "Open Source • Private • Multilingual",
        hero_title_top: "Make AI",
        hero_title_focus: "part of your daily life",
        hero_sub_1: "Nola is open-source and lets you use your own LLM endpoint and key.",
        hero_sub_2: "Your personal data is local-first, so control stays with you.",
        hero_cta_primary: "Get Started",
        hero_cta_secondary: "Explore features",
        phone_msg_in: "Which model do you want to use today?",
        phone_msg_out: "Switch to my endpoint, send summary at 9 AM",
        phone_card_title: "Task created",
        phone_card_meta: "Key stored locally · Daily 09:00",
        phone_card_tag: "Private ✓",
        stat_1: "Platform roadmap<br>iOS · Web · Android · CLI",
        stat_2: "Automation<br>Less repetitive work",
        stat_3: "Local-first<br>Your data, your control",
        stat_4: "Multilingual UX<br>Common languages supported",
        features_kicker: "What makes Nola different",
        features_title_main: "Less setup, faster outcomes.",
        features_title_sub: "Built for user experience, not complexity.",
        f1_title: "A customizable AI assistant",
        f1_desc: "Use your own LLM endpoint and key with the workflow you already prefer.",
        f1_b1: "Custom endpoint",
        f1_b2: "Bring your own key",
        f1_b3: "Multi-session chat",
        f2_title: "Automation that actually helps",
        f2_desc: "Set once, run on schedule, and get notified with useful summaries.",
        f3_title: "Results you can continue from",
        f3_desc: "Get notified, review history, and keep asking from the same context.",
        f4_title: "Local-first data strategy",
        f4_desc: "Personal settings and data are local-first by default.",
        f5_title: "Localized for common languages",
        f5_desc: "Use Nola in the language you are comfortable with.",
        flow_kicker: "How it works",
        flow_title_main: "Start in three steps.",
        flow_title_sub: "Simple onboarding, practical results.",
        step1_title: "Out of the box. Connect your model and go.",
        step1_desc: "No extra onboarding steps. Add your endpoint and key, then start immediately.",
        mock_r1_l: "Experience",
        mock_r1_v: "Out of the box",
        mock_r2_l: "Platform",
        mock_r2_v: "iOS / Web",
        mock_r3_l: "Endpoint",
        mock_r3_v: "Custom Endpoint",
        mock_r4_l: "Key",
        mock_r4_v: "Local Only",
        mock_btn: "Start",
        step2_title: "Let Nola handle repetitive work",
        step2_desc: "From collection to summary and reminders, routine tasks run automatically.",
        node_input: "Input",
        node_process: "Process",
        node_output: "Result",
        step3_title: "Get results and keep going",
        step3_desc: "Receive updates, review history, and continue the conversation anytime.",
        notify_sub: "Your task is complete",
        notify_body: "Summary is ready. Tap to view and ask follow-up questions.",
        platform_kicker: "Roadmap",
        platform_title: "Open-source roadmap, continuously expanding.",
        p1_tag: "Now",
        p1_title: "iOS + Web",
        p1_desc: "Available today with mobile and browser experiences.",
        p2_tag: "Next",
        p2_title: "Android + CLI",
        p2_desc: "Planned support for Android and CLI to fit more workflows.",
        p3_tag: "Open Source",
        p3_title: "Community built",
        p3_desc: "Nola is open-source. Contributions and feedback are welcome.",
        pill_1: "Open Source",
        pill_2: "Custom Endpoint",
        pill_3: "Bring Your Own Key",
        pill_4: "Local-first",
        pill_5: "iOS",
        pill_6: "Web",
        pill_7: "Android (Planned)",
        pill_8: "CLI (Planned)",
        faq_title: "Questions you may ask",
        faq_q1: "Is Nola open-source?",
        faq_a1: "Yes. Nola is open-source and open to community contributions.",
        faq_q2: "Can I set my own LLM endpoint and key?",
        faq_a2: "Yes. You can configure your own endpoint and key freely.",
        faq_q3: "Where is my data stored?",
        faq_a3: "Data is local-first by default, so you keep control.",
        faq_q4: "Beyond iOS and Web, what is next?",
        faq_a4: "Android and CLI are planned next.",
        faq_q5: "Does it support multiple languages?",
        faq_a5: "Yes. The website and product support common languages.",
        cta_title: "Ready to try Nola?",
        cta_desc: "Tell us your scenario and we will get back to you shortly.",
        cta_contact: "Contact us",
        cta_back: "Back to top",
        footer_meta: "© 2026 Nola. Built with community.",
        footer_overview: "Overview",
        footer_features: "Features",
        footer_contact: "Contact",
        nav_pricing: "Deploy",
        pricing_kicker: "Deploy",
        pricing_title: "Run Nola your way.",
        pricing_subtitle: "Open source, self-hostable, and ready for your own model endpoint.",
        plan_free_name: "Open Source",
        plan_free_price: "$0",
        plan_free_period: "forever",
        plan_free_desc: "Bring your own LLM endpoint and key. Data stays local.",
        plan_free_features: "<li>Bring your own LLM key &amp; endpoint</li><li>Local-first data storage</li><li>Open source &amp; self-hostable</li><li>Community support</li>",
        plan_free_cta: "View on GitHub",
        plan_pro_name: "Self-hosted",
        plan_pro_price: "$0",
        plan_pro_period: "optional",
        plan_pro_desc: "Run your own backend and connect your own AI provider.",
        plan_pro_features: "<li>Custom backend deployment</li><li>Scheduled tasks &amp; notifications</li><li>Multi-session chat history</li><li>Bring your own model endpoint</li>",
        plan_pro_cta: "View deployment guide",
        pricing_note: "Use your own infrastructure and model provider.",
    };

    const ZH = {
        title: "Nola · 开源个人助手",
        meta_description: "Nola 是开源的个人智能助理：可自定义 LLM 端点和 Key，数据默认本地保存，支持多语言与跨平台。",
        og_description: "开源、可自定义、隐私优先，让 AI 成为你的日常助手。",
        nav_overview: "概览",
        nav_features: "能力",
        nav_flow: "流程",
        nav_platform: "平台",
        nav_faq: "FAQ",
        nav_contact_chip: "联系体验 →",
        nav_contact: "联系我们",
        lang_label: "语言",
        hero_eyebrow: "开源 · 本地优先 · 多语言",
        hero_title_top: "让 AI 成为",
        hero_title_focus: "你日常的一部分",
        hero_sub_1: "Nola 是开源助手，你可以自由配置 LLM 端点和 Key。",
        hero_sub_2: "你的个人数据默认保存在本地，体验由你掌控。",
        hero_cta_primary: "立即体验",
        hero_cta_secondary: "了解功能",
        phone_msg_in: "今天要用哪个模型？",
        phone_msg_out: "切到我的端点，9 点推送摘要",
        phone_card_title: "任务已创建",
        phone_card_meta: "Key 本地保存 · 每日 09:00",
        phone_card_tag: "Private ✓",
        stat_1: "平台路线<br>iOS · Web · Android · CLI",
        stat_2: "自动化执行<br>让重复工作消失",
        stat_3: "本地优先<br>数据由你掌控",
        stat_4: "多语言体验<br>覆盖常见语言",
        features_kicker: "Nola 的核心优势",
        features_title_main: "更少设置，更快得到结果。",
        features_title_sub: "专注体验，而不是复杂配置。",
        f1_title: "可自定义的 AI 助手",
        f1_desc: "你可以连接自己的 LLM 端点和 Key，用你熟悉的模型工作。",
        f1_b1: "自定义端点",
        f1_b2: "自定义 Key",
        f1_b3: "多会话管理",
        f2_title: "定时任务自动完成",
        f2_desc: "一次设置，按时执行，自动汇总并提醒。",
        f3_title: "结果可持续追问",
        f3_desc: "收到结果后可继续对话，让每一次输出都能延展。",
        f4_title: "本地优先的数据策略",
        f4_desc: "配置与个人数据默认保存在本地，隐私边界更清晰。",
        f5_title: "全语言本地化",
        f5_desc: "支持常见语言，让全球用户都能自然上手。",
        flow_kicker: "使用方式",
        flow_title_main: "三步开始使用。",
        flow_title_sub: "体验优先，学习成本更低。",
        step1_title: "开箱即用，连接模型就开始",
        step1_desc: "无需额外配置流程，连上你的模型端点与 Key 即可直接使用。",
        mock_r1_l: "体验",
        mock_r1_v: "开箱即用",
        mock_r2_l: "平台",
        mock_r2_v: "iOS / Web",
        mock_r3_l: "端点",
        mock_r3_v: "Custom Endpoint",
        mock_r4_l: "Key",
        mock_r4_v: "Local Only",
        mock_btn: "开始使用",
        step2_title: "把重复任务交给 Nola",
        step2_desc: "从收集、整理到提醒，重复流程自动完成。",
        node_input: "输入",
        node_process: "处理",
        node_output: "结果",
        step3_title: "结果送达，继续对话",
        step3_desc: "任务完成即时提醒，历史可追踪，上下文可延续。",
        notify_sub: "你的任务已完成",
        notify_body: "摘要已生成，点击查看并继续追问。",
        platform_kicker: "路线图",
        platform_title: "开源路线，持续扩展。",
        p1_tag: "当前",
        p1_title: "iOS + Web",
        p1_desc: "当前已可用的双入口，覆盖移动端与浏览器场景。",
        p2_tag: "下一步",
        p2_title: "Android + CLI",
        p2_desc: "后续将支持 Android 与 CLI，覆盖更广使用场景。",
        p3_tag: "开源",
        p3_title: "社区共建",
        p3_desc: "Nola 是开源项目，欢迎贡献代码与建议。",
        pill_1: "开源",
        pill_2: "自定义端点",
        pill_3: "自定义 Key",
        pill_4: "本地优先",
        pill_5: "iOS",
        pill_6: "Web",
        pill_7: "Android（规划中）",
        pill_8: "CLI（规划中）",
        faq_title: "你关心的问题",
        faq_q1: "Nola 是开源项目吗？",
        faq_a1: "是。Nola 是开源项目，欢迎查看、使用并参与共建。",
        faq_q2: "可以设置自己的 LLM 端点和 Key 吗？",
        faq_a2: "可以。你可以自由设置自己的端点和 Key。",
        faq_q3: "数据存在哪里？",
        faq_a3: "默认保存在本地。你对自己的数据拥有完整控制权。",
        faq_q4: "除了 iOS 和 Web，后面还有什么？",
        faq_a4: "后续将支持 Android 和 CLI。",
        faq_q5: "支持多语言吗？",
        faq_a5: "支持。官网与产品会覆盖常见语言本地化。",
        cta_title: "准备开始了吗？",
        cta_desc: "告诉我们你的使用场景，我们会尽快回复。",
        cta_contact: "联系我们",
        cta_back: "回到顶部",
        footer_meta: "© 2026 Nola. 由社区共同构建。",
        footer_overview: "概览",
        footer_features: "能力",
        footer_contact: "联系",
        nav_pricing: "部署",
        pricing_kicker: "部署",
        pricing_title: "按你的方式运行 Nola。",
        pricing_subtitle: "开源、自托管，并可接入自己的模型端点。",
        plan_free_name: "开源版",
        plan_free_price: "免费",
        plan_free_period: "永久",
        plan_free_desc: "使用自己的 LLM 端点与 Key，数据本地保存。",
        plan_free_features: "<li>自定义 LLM Key 与端点</li><li>本地优先数据存储</li><li>开源可自托管</li><li>社区支持</li>",
        plan_free_cta: "前往 GitHub",
        plan_pro_name: "自托管",
        plan_pro_price: "¥0",
        plan_pro_period: "可选",
        plan_pro_desc: "运行自己的后端，接入自己的 AI 服务。",
        plan_pro_features: "<li>自定义后端部署</li><li>定时任务与通知</li><li>多会话历史记录</li><li>接入自己的模型端点</li>",
        plan_pro_cta: "查看部署说明",
        pricing_note: "使用自己的基础设施和模型服务。",
    };

    const ES = {
        title: "Nola · Asistente personal de código abierto",
        nav_overview: "Resumen",
        nav_features: "Funciones",
        nav_flow: "Flujo",
        nav_platform: "Plataformas",
        nav_contact: "Contacto",
        nav_contact_chip: "Probar Nola →",
        hero_eyebrow: "Código abierto • Privado • Multilingüe",
        hero_title_top: "Haz que la IA sea",
        hero_title_focus: "parte de tu día a día",
        hero_sub_1: "Nola es de código abierto y te permite usar tu propio endpoint y clave LLM.",
        hero_sub_2: "Tus datos personales se guardan localmente por defecto.",
        hero_cta_primary: "Comenzar",
        hero_cta_secondary: "Ver funciones",
        stat_1: "Ruta de plataformas<br>iOS · Web · Android · CLI",
        stat_3: "Local-first<br>Tú controlas tus datos",
        stat_4: "Experiencia multilingüe<br>Idiomas comunes incluidos",
        features_title_main: "Menos configuración, más resultados.",
        f1_title: "Asistente de IA personalizable",
        f4_title: "Estrategia local-first",
        f5_title: "Localización multilingüe",
        platform_title: "Hoja de ruta abierta y en expansión.",
        faq_title: "Preguntas frecuentes",
        faq_q1: "¿Nola es de código abierto?",
        faq_a1: "Sí. Nola es de código abierto y acepta contribuciones.",
        faq_q2: "¿Puedo configurar mi endpoint y clave LLM?",
        faq_a2: "Sí. Puedes usar tu propio endpoint y clave.",
        faq_q3: "¿Dónde se guardan mis datos?",
        faq_a3: "Por defecto, en local.",
        faq_q4: "¿Qué viene después de iOS y Web?",
        faq_a4: "Android y CLI están planificados.",
        faq_q5: "¿Tiene soporte multilingüe?",
        faq_a5: "Sí. Sitio y producto soportan idiomas comunes.",
        cta_title: "¿Listo para probar Nola?",
        cta_contact: "Contáctanos",
        footer_overview: "Resumen",
        footer_features: "Funciones",
        footer_contact: "Contacto",
        nav_pricing: "Despliegue",
        pricing_kicker: "Despliegue",
        pricing_title: "Ejecuta Nola a tu manera.",
        pricing_subtitle: "Código abierto, autohospedable y listo para tu endpoint de modelo.",
        plan_free_name: "Código Abierto",
        plan_free_price: "$0",
        plan_free_period: "siempre",
        plan_free_desc: "Usa tu propio endpoint y clave LLM. Datos locales.",
        plan_free_features: "<li>Tu propia clave LLM y endpoint</li><li>Almacenamiento local</li><li>Código abierto y auto-hospedable</li><li>Soporte comunitario</li>",
        plan_free_cta: "Ver en GitHub",
        plan_pro_name: "Autohospedado",
        plan_pro_price: "$0",
        plan_pro_period: "opcional",
        plan_pro_desc: "Ejecuta tu propio backend y conecta tu proveedor de IA.",
        plan_pro_features: "<li>Despliegue backend personalizado</li><li>Tareas programadas y notificaciones</li><li>Historial de múltiples sesiones</li><li>Trae tu propio endpoint de modelo</li>",
        plan_pro_cta: "Ver guía de despliegue",
        pricing_note: "Usa tu propia infraestructura y proveedor de modelos.",
    };

    const FR = {
        title: "Nola · Assistant personnel open source",
        nav_overview: "Aperçu",
        nav_features: "Fonctionnalités",
        nav_flow: "Parcours",
        nav_platform: "Plateformes",
        nav_contact: "Contact",
        nav_contact_chip: "Essayer Nola →",
        hero_eyebrow: "Open source • Privé • Multilingue",
        hero_title_top: "Faites de l'IA",
        hero_title_focus: "une partie de votre quotidien",
        hero_sub_1: "Nola est open source et prend en charge votre endpoint et clé LLM.",
        hero_sub_2: "Vos données personnelles restent locales par défaut.",
        hero_cta_primary: "Commencer",
        hero_cta_secondary: "Découvrir",
        stat_1: "Feuille de route<br>iOS · Web · Android · CLI",
        stat_3: "Local-first<br>Vos données, votre contrôle",
        stat_4: "Expérience multilingue<br>Langues courantes prises en charge",
        features_title_main: "Moins de configuration, plus de résultats.",
        f1_title: "Assistant IA personnalisable",
        f4_title: "Stratégie local-first",
        f5_title: "Localisation multilingue",
        platform_title: "Feuille de route open source en expansion.",
        faq_title: "Questions fréquentes",
        faq_q1: "Nola est-il open source ?",
        faq_q2: "Puis-je configurer mon endpoint et ma clé LLM ?",
        faq_q3: "Où sont stockées mes données ?",
        faq_q4: "Après iOS et Web, quelle suite ?",
        faq_q5: "Le multilingue est-il pris en charge ?",
        cta_title: "Prêt à essayer Nola ?",
        cta_contact: "Nous contacter",
        nav_pricing: "Déploiement",
        pricing_kicker: "Déploiement",
        pricing_title: "Exécutez Nola à votre façon.",
        pricing_subtitle: "Open source, auto-hébergeable et prêt pour votre endpoint de modèle.",
        plan_free_name: "Open Source",
        plan_free_price: "0 €",
        plan_free_period: "pour toujours",
        plan_free_desc: "Utilisez votre propre endpoint et clé LLM. Données locales.",
        plan_free_features: "<li>Votre propre clé LLM &amp; endpoint</li><li>Stockage local en priorité</li><li>Open source &amp; auto-hébergeable</li><li>Support communautaire</li>",
        plan_free_cta: "Voir sur GitHub",
        plan_pro_name: "Auto-hébergé",
        plan_pro_price: "0 €",
        plan_pro_period: "optionnel",
        plan_pro_desc: "Exécutez votre propre backend et connectez votre fournisseur IA.",
        plan_pro_features: "<li>Déploiement backend personnalisé</li><li>Tâches planifiées et notifications</li><li>Historique multi-sessions</li><li>Utilisez votre propre endpoint de modèle</li>",
        plan_pro_cta: "Voir le guide de déploiement",
        pricing_note: "Utilisez votre propre infrastructure et fournisseur de modèles.",
    };

    const DE = {
        title: "Nola · Open-Source-Personalassistent",
        nav_overview: "Überblick",
        nav_features: "Funktionen",
        nav_flow: "Ablauf",
        nav_platform: "Plattformen",
        nav_contact: "Kontakt",
        nav_contact_chip: "Nola testen →",
        hero_eyebrow: "Open Source • Privat • Mehrsprachig",
        hero_title_top: "Mach KI",
        hero_title_focus: "zu deinem Alltag",
        hero_sub_1: "Nola ist Open Source und unterstützt deinen eigenen LLM-Endpunkt und Key.",
        hero_sub_2: "Persönliche Daten bleiben standardmäßig lokal.",
        hero_cta_primary: "Loslegen",
        hero_cta_secondary: "Funktionen ansehen",
        stat_1: "Plattform-Roadmap<br>iOS · Web · Android · CLI",
        stat_3: "Local-first<br>Deine Daten, deine Kontrolle",
        stat_4: "Mehrsprachige UX<br>Gängige Sprachen unterstützt",
        features_title_main: "Weniger Setup, schnellere Ergebnisse.",
        f1_title: "Anpassbarer KI-Assistent",
        f4_title: "Local-first Datenstrategie",
        f5_title: "Mehrsprachige Lokalisierung",
        platform_title: "Open-Source-Roadmap, die weiter wächst.",
        faq_title: "Häufige Fragen",
        faq_q1: "Ist Nola Open Source?",
        faq_q2: "Kann ich eigenen LLM-Endpunkt und Key nutzen?",
        faq_q3: "Wo werden meine Daten gespeichert?",
        faq_q4: "Was kommt nach iOS und Web?",
        faq_q5: "Gibt es Mehrsprachigkeit?",
        cta_title: "Bereit für Nola?",
        cta_contact: "Kontakt",
        nav_pricing: "Bereitstellen",
        pricing_kicker: "Bereitstellen",
        pricing_title: "Nutze Nola auf deine Weise.",
        pricing_subtitle: "Open Source, selbst hostbar und bereit für deinen Modell-Endpunkt.",
        plan_free_name: "Open Source",
        plan_free_price: "0 €",
        plan_free_period: "für immer",
        plan_free_desc: "Nutze deinen eigenen LLM-Endpunkt und Key. Daten bleiben lokal.",
        plan_free_features: "<li>Eigener LLM-Key &amp; Endpunkt</li><li>Lokale Datenspeicherung</li><li>Open Source &amp; selbst hostbar</li><li>Community-Support</li>",
        plan_free_cta: "Auf GitHub ansehen",
        plan_pro_name: "Selbst gehostet",
        plan_pro_price: "0 €",
        plan_pro_period: "optional",
        plan_pro_desc: "Betreibe dein eigenes Backend und verbinde deinen KI-Anbieter.",
        plan_pro_features: "<li>Eigenes Backend-Deployment</li><li>Geplante Aufgaben &amp; Benachrichtigungen</li><li>Multi-Session-Verlauf</li><li>Eigener Modell-Endpunkt</li>",
        plan_pro_cta: "Deployment-Anleitung ansehen",
        pricing_note: "Nutze deine eigene Infrastruktur und deinen Modellanbieter.",
    };

    const JA = {
        title: "Nola · オープンソースのパーソナルアシスタント",
        nav_overview: "概要",
        nav_features: "機能",
        nav_flow: "使い方",
        nav_platform: "プラットフォーム",
        nav_contact: "お問い合わせ",
        nav_contact_chip: "Nola を試す →",
        hero_eyebrow: "オープンソース • プライベート • 多言語",
        hero_title_top: "AI を",
        hero_title_focus: "日常の一部に",
        hero_sub_1: "Nola はオープンソース。LLM のエンドポイントとキーを自由に設定できます。",
        hero_sub_2: "個人データはデフォルトでローカル保存です。",
        hero_cta_primary: "はじめる",
        hero_cta_secondary: "機能を見る",
        stat_1: "プラットフォーム予定<br>iOS · Web · Android · CLI",
        stat_3: "ローカル優先<br>データは自分で管理",
        stat_4: "多言語体験<br>主要言語に対応",
        features_title_main: "設定は少なく、結果は速く。",
        f1_title: "カスタマイズ可能な AI アシスタント",
        f4_title: "ローカル優先データ戦略",
        f5_title: "多言語ローカライズ",
        platform_title: "オープンソースのロードマップを継続拡張。",
        faq_title: "よくある質問",
        faq_q1: "Nola はオープンソースですか？",
        faq_q2: "LLM のエンドポイントとキーを自分で設定できますか？",
        faq_q3: "データはどこに保存されますか？",
        faq_q4: "iOS と Web の次は？",
        faq_q5: "多言語対応ですか？",
        cta_title: "Nola を始めますか？",
        cta_contact: "お問い合わせ",
        nav_pricing: "デプロイ",
        pricing_kicker: "デプロイ",
        pricing_title: "Nola を自分の環境で実行。",
        pricing_subtitle: "オープンソースで自己ホストでき、独自のモデルエンドポイントに接続できます。",
        plan_free_name: "オープンソース",
        plan_free_price: "¥0",
        plan_free_period: "永久無料",
        plan_free_desc: "自分のLLMエンドポイントとキーを使用。データはローカルに保存。",
        plan_free_features: "<li>自分のLLMキー &amp; エンドポイント</li><li>ローカル優先データ保存</li><li>オープンソース &amp; 自己ホスト可能</li><li>コミュニティサポート</li>",
        plan_free_cta: "GitHubで見る",
        plan_pro_name: "自己ホスト",
        plan_pro_price: "¥0",
        plan_pro_period: "任意",
        plan_pro_desc: "自分のバックエンドを実行し、任意の AI プロバイダーに接続します。",
        plan_pro_features: "<li>カスタムバックエンドのデプロイ</li><li>定期タスクと通知</li><li>マルチセッション履歴</li><li>独自のモデルエンドポイント</li>",
        plan_pro_cta: "デプロイ手順を見る",
        pricing_note: "自分のインフラとモデルプロバイダーを使用します。",
    };

    const KO = {
        title: "Nola · 오픈소스 개인 비서",
        nav_overview: "개요",
        nav_features: "기능",
        nav_flow: "흐름",
        nav_platform: "플랫폼",
        nav_contact: "문의",
        nav_contact_chip: "Nola 체험 →",
        hero_eyebrow: "오픈소스 • 프라이빗 • 다국어",
        hero_title_top: "AI를",
        hero_title_focus: "일상의 일부로",
        hero_sub_1: "Nola는 오픈소스이며 LLM 엔드포인트와 키를 직접 설정할 수 있습니다.",
        hero_sub_2: "개인 데이터는 기본적으로 로컬에 저장됩니다.",
        hero_cta_primary: "시작하기",
        hero_cta_secondary: "기능 보기",
        stat_1: "플랫폼 로드맵<br>iOS · Web · Android · CLI",
        stat_3: "로컬 우선<br>데이터는 사용자가 제어",
        stat_4: "다국어 UX<br>주요 언어 지원",
        features_title_main: "설정은 줄이고 결과는 빠르게.",
        f1_title: "커스터마이즈 가능한 AI 비서",
        f4_title: "로컬 우선 데이터 전략",
        f5_title: "다국어 현지화",
        platform_title: "오픈소스 로드맵을 계속 확장합니다.",
        faq_title: "자주 묻는 질문",
        faq_q1: "Nola는 오픈소스인가요?",
        faq_q2: "내 LLM 엔드포인트와 키를 설정할 수 있나요?",
        faq_q3: "데이터는 어디에 저장되나요?",
        faq_q4: "iOS와 Web 다음은 무엇인가요?",
        faq_q5: "다국어를 지원하나요?",
        cta_title: "Nola를 시작해볼까요?",
        cta_contact: "문의하기",
        nav_pricing: "배포",
        pricing_kicker: "배포",
        pricing_title: "원하는 방식으로 Nola를 실행하세요.",
        pricing_subtitle: "오픈소스, 자체 호스팅, 자체 모델 엔드포인트 연결을 지원합니다.",
        plan_free_name: "오픈소스",
        plan_free_price: "₩0",
        plan_free_period: "영구 무료",
        plan_free_desc: "자신의 LLM 키와 엔드포인트 사용. 데이터는 로컬 저장.",
        plan_free_features: "<li>자신의 LLM 키 &amp; 엔드포인트</li><li>로컬 우선 데이터 저장</li><li>오픈소스 &amp; 자체 호스팅 가능</li><li>커뮤니티 지원</li>",
        plan_free_cta: "GitHub에서 보기",
        plan_pro_name: "자체 호스팅",
        plan_pro_price: "₩0",
        plan_pro_period: "선택",
        plan_pro_desc: "자체 백엔드를 실행하고 원하는 AI 제공자를 연결하세요.",
        plan_pro_features: "<li>맞춤 백엔드 배포</li><li>예약 작업 및 알림</li><li>멀티 세션 채팅 기록</li><li>자체 모델 엔드포인트</li>",
        plan_pro_cta: "배포 가이드 보기",
        pricing_note: "자체 인프라와 모델 제공자를 사용합니다.",
    };

    const RU = {
        title: "Nola · Open-source персональный ассистент",
        nav_overview: "Обзор",
        nav_features: "Функции",
        nav_flow: "Сценарий",
        nav_platform: "Платформы",
        nav_contact: "Контакты",
        nav_contact_chip: "Попробовать Nola →",
        hero_eyebrow: "Open source • Приватность • Мультиязычность",
        hero_title_top: "Сделайте ИИ",
        hero_title_focus: "частью вашего дня",
        hero_sub_1: "Nola — open-source и поддерживает ваш собственный LLM endpoint и key.",
        hero_sub_2: "Личные данные по умолчанию хранятся локально.",
        hero_cta_primary: "Начать",
        hero_cta_secondary: "Смотреть функции",
        stat_1: "Платформенный план<br>iOS · Web · Android · CLI",
        stat_3: "Local-first<br>Ваши данные под вашим контролем",
        stat_4: "Мультиязычный UX<br>Поддержка популярных языков",
        features_title_main: "Меньше настройки, быстрее результат.",
        f1_title: "Настраиваемый ИИ-ассистент",
        f4_title: "Local-first стратегия данных",
        f5_title: "Мультиязычная локализация",
        platform_title: "Open-source дорожная карта продолжает расти.",
        faq_title: "Частые вопросы",
        faq_q1: "Nola — open-source?",
        faq_q2: "Можно задать свой endpoint и key LLM?",
        faq_q3: "Где хранятся данные?",
        faq_q4: "Что после iOS и Web?",
        faq_q5: "Есть поддержка разных языков?",
        cta_title: "Готовы попробовать Nola?",
        cta_contact: "Связаться",
        nav_pricing: "Развертывание",
        pricing_kicker: "Развертывание",
        pricing_title: "Запускайте Nola по-своему.",
        pricing_subtitle: "Open source, самостоятельный хостинг и подключение своего модельного endpoint.",
        plan_free_name: "Open Source",
        plan_free_price: "$0",
        plan_free_period: "навсегда",
        plan_free_desc: "Используйте свой LLM-endpoint и key. Данные хранятся локально.",
        plan_free_features: "<li>Свой LLM-ключ &amp; endpoint</li><li>Локальное хранение данных</li><li>Open source &amp; самохостинг</li><li>Поддержка сообщества</li>",
        plan_free_cta: "Смотреть на GitHub",
        plan_pro_name: "Самостоятельный хостинг",
        plan_pro_price: "$0",
        plan_pro_period: "опционально",
        plan_pro_desc: "Запустите свой backend и подключите своего AI-провайдера.",
        plan_pro_features: "<li>Собственное развертывание backend</li><li>Запланированные задачи и уведомления</li><li>История нескольких сессий</li><li>Свой модельный endpoint</li>",
        plan_pro_cta: "Открыть руководство",
        pricing_note: "Используйте свою инфраструктуру и модельного провайдера.",
    };

    const PTBR = {
        title: "Nola · Assistente pessoal open source",
        nav_overview: "Visão geral",
        nav_features: "Recursos",
        nav_flow: "Fluxo",
        nav_platform: "Plataformas",
        nav_contact: "Contato",
        nav_contact_chip: "Experimentar Nola →",
        hero_eyebrow: "Open source • Privado • Multilíngue",
        hero_title_top: "Transforme IA",
        hero_title_focus: "em parte do seu dia",
        hero_sub_1: "Nola é open source e permite usar seu endpoint e key de LLM.",
        hero_sub_2: "Seus dados pessoais ficam locais por padrão.",
        hero_cta_primary: "Começar",
        hero_cta_secondary: "Ver recursos",
        stat_1: "Roteiro de plataforma<br>iOS · Web · Android · CLI",
        stat_3: "Local-first<br>Seus dados sob seu controle",
        stat_4: "Experiência multilíngue<br>Idiomas comuns suportados",
        features_title_main: "Menos configuração, resultados mais rápidos.",
        f1_title: "Assistente de IA personalizável",
        f4_title: "Estratégia local-first",
        f5_title: "Localização multilíngue",
        platform_title: "Roteiro open source em expansão contínua.",
        faq_title: "Perguntas frequentes",
        faq_q1: "Nola é open source?",
        faq_q2: "Posso definir meu endpoint e key de LLM?",
        faq_q3: "Onde meus dados ficam?",
        faq_q4: "Depois de iOS e Web, o que vem?",
        faq_q5: "Há suporte a vários idiomas?",
        cta_title: "Pronto para usar Nola?",
        cta_contact: "Fale conosco",
        nav_pricing: "Implantar",
        pricing_kicker: "Implantar",
        pricing_title: "Execute o Nola do seu jeito.",
        pricing_subtitle: "Código aberto, auto-hospedável e pronto para seu endpoint de modelo.",
        plan_free_name: "Código Aberto",
        plan_free_price: "R$0",
        plan_free_period: "para sempre",
        plan_free_desc: "Use sua própria chave e endpoint LLM. Dados ficam locais.",
        plan_free_features: "<li>Sua própria chave LLM &amp; endpoint</li><li>Armazenamento local</li><li>Código aberto e auto-hospedável</li><li>Suporte da comunidade</li>",
        plan_free_cta: "Ver no GitHub",
        plan_pro_name: "Auto-hospedado",
        plan_pro_price: "R$0",
        plan_pro_period: "opcional",
        plan_pro_desc: "Execute seu próprio backend e conecte seu provedor de IA.",
        plan_pro_features: "<li>Implantação backend personalizada</li><li>Tarefas programadas e notificações</li><li>Histórico de múltiplas sessões</li><li>Seu próprio endpoint de modelo</li>",
        plan_pro_cta: "Ver guia de implantação",
        pricing_note: "Use sua própria infraestrutura e provedor de modelos.",
    };

    const AR = {
        title: "Nola · مساعد شخصي مفتوح المصدر",
        nav_overview: "نظرة عامة",
        nav_features: "المزايا",
        nav_flow: "التدفق",
        nav_platform: "المنصات",
        nav_contact: "تواصل",
        nav_contact_chip: "جرّب Nola →",
        hero_eyebrow: "مفتوح المصدر • خصوصية • متعدد اللغات",
        hero_title_top: "اجعل الذكاء الاصطناعي",
        hero_title_focus: "جزءاً من يومك",
        hero_sub_1: "Nola مفتوح المصدر ويدعم إعداد نقطة نهاية ومفتاح LLM الخاص بك.",
        hero_sub_2: "بياناتك الشخصية تُحفظ محلياً بشكل افتراضي.",
        hero_cta_primary: "ابدأ الآن",
        hero_cta_secondary: "استكشف المزايا",
        stat_1: "خارطة المنصات<br>iOS · Web · Android · CLI",
        stat_3: "محلي أولاً<br>بياناتك تحت سيطرتك",
        stat_4: "تجربة متعددة اللغات<br>دعم اللغات الشائعة",
        features_title_main: "إعداد أقل ونتائج أسرع.",
        f1_title: "مساعد ذكاء اصطناعي قابل للتخصيص",
        f4_title: "استراتيجية بيانات محلية أولاً",
        f5_title: "تعريب متعدد اللغات",
        platform_title: "خارطة طريق مفتوحة المصدر وتتوسع باستمرار.",
        faq_title: "الأسئلة الشائعة",
        faq_q1: "هل Nola مفتوح المصدر؟",
        faq_q2: "هل يمكنني ضبط نقطة نهاية ومفتاح LLM الخاص بي؟",
        faq_q3: "أين يتم حفظ البيانات؟",
        faq_q4: "ما التالي بعد iOS وWeb؟",
        faq_q5: "هل يدعم تعدد اللغات؟",
        cta_title: "هل أنت مستعد لتجربة Nola؟",
        cta_contact: "تواصل معنا",
        nav_pricing: "النشر",
        pricing_kicker: "النشر",
        pricing_title: "شغّل Nola بطريقتك.",
        pricing_subtitle: "مفتوح المصدر وقابل للاستضافة الذاتية وجاهز لربطه بنقطة نموذجك.",
        plan_free_name: "مفتوح المصدر",
        plan_free_price: "$0",
        plan_free_period: "للأبد",
        plan_free_desc: "استخدم نقطة نهاية ومفتاح LLM الخاص بك. البيانات محلية.",
        plan_free_features: "<li>مفتاح LLM ونقطة نهاية خاصة بك</li><li>تخزين البيانات محلياً</li><li>مفتوح المصدر وقابل للاستضافة الذاتية</li><li>دعم المجتمع</li>",
        plan_free_cta: "عرض على GitHub",
        plan_pro_name: "استضافة ذاتية",
        plan_pro_price: "$0",
        plan_pro_period: "اختياري",
        plan_pro_desc: "شغّل backend الخاص بك واربط مزود الذكاء الاصطناعي الذي تختاره.",
        plan_pro_features: "<li>نشر backend مخصص</li><li>مهام مجدولة وإشعارات</li><li>سجل محادثات متعدد الجلسات</li><li>نقطة نموذج خاصة بك</li>",
        plan_pro_cta: "عرض دليل النشر",
        pricing_note: "استخدم بنيتك التحتية ومزود النموذج الخاص بك.",
    };

    const HI = {
        title: "Nola · ओपन-सोर्स पर्सनल असिस्टेंट",
        nav_overview: "ओवरव्यू",
        nav_features: "फीचर्स",
        nav_flow: "प्रवाह",
        nav_platform: "प्लेटफ़ॉर्म",
        nav_contact: "संपर्क",
        nav_contact_chip: "Nola आज़माएँ →",
        hero_eyebrow: "ओपन सोर्स • प्राइवेट • बहुभाषी",
        hero_title_top: "AI को बनाइए",
        hero_title_focus: "अपने रोज़मर्रा का हिस्सा",
        hero_sub_1: "Nola ओपन-सोर्स है और आप अपना LLM endpoint और key सेट कर सकते हैं।",
        hero_sub_2: "आपका व्यक्तिगत डेटा डिफ़ॉल्ट रूप से लोकल में रहता है।",
        hero_cta_primary: "शुरू करें",
        hero_cta_secondary: "फीचर्स देखें",
        stat_1: "प्लेटफ़ॉर्म रोडमैप<br>iOS · Web · Android · CLI",
        stat_3: "लोकल-फर्स्ट<br>डेटा आपके नियंत्रण में",
        stat_4: "बहुभाषी अनुभव<br>सामान्य भाषाओं का समर्थन",
        features_title_main: "कम सेटअप, तेज़ परिणाम।",
        f1_title: "कस्टमाइज़ेबल AI असिस्टेंट",
        f4_title: "लोकल-फर्स्ट डेटा रणनीति",
        f5_title: "बहुभाषी स्थानीयकरण",
        platform_title: "ओपन-सोर्स रोडमैप लगातार बढ़ रहा है।",
        faq_title: "अक्सर पूछे जाने वाले सवाल",
        faq_q1: "क्या Nola ओपन-सोर्स है?",
        faq_q2: "क्या मैं अपना LLM endpoint और key सेट कर सकता हूँ?",
        faq_q3: "डेटा कहाँ स्टोर होता है?",
        faq_q4: "iOS और Web के बाद क्या आएगा?",
        faq_q5: "क्या बहुभाषी समर्थन है?",
        cta_title: "Nola आज़माने के लिए तैयार हैं?",
        cta_contact: "संपर्क करें",
        nav_pricing: "डिप्लॉय",
        pricing_kicker: "डिप्लॉय",
        pricing_title: "Nola को अपने तरीके से चलाएं।",
        pricing_subtitle: "ओपन सोर्स, self-hostable, और आपके model endpoint के लिए तैयार।",
        plan_free_name: "ओपन सोर्स",
        plan_free_price: "₹0",
        plan_free_period: "हमेशा के लिए",
        plan_free_desc: "अपनी LLM key और endpoint का उपयोग करें। डेटा लोकल रहता है।",
        plan_free_features: "<li>अपनी LLM key &amp; endpoint</li><li>लोकल-फर्स्ट डेटा स्टोरेज</li><li>ओपन सोर्स &amp; सेल्फ-होस्टेबल</li><li>कम्युनिटी सपोर्ट</li>",
        plan_free_cta: "GitHub पर देखें",
        plan_pro_name: "Self-hosted",
        plan_pro_price: "₹0",
        plan_pro_period: "वैकल्पिक",
        plan_pro_desc: "अपना backend चलाएं और अपना AI provider जोड़ें।",
        plan_pro_features: "<li>कस्टम backend deployment</li><li>शेड्यूल्ड टास्क और नोटिफिकेशन</li><li>मल्टी-सेशन चैट हिस्ट्री</li><li>अपना model endpoint जोड़ें</li>",
        plan_pro_cta: "Deployment guide देखें",
        pricing_note: "अपना infrastructure और model provider उपयोग करें.",
    };

    const TRANSLATIONS = {
        "en": EN,
        "zh-CN": { ...EN, ...ZH },
        "es": { ...EN, ...ES },
        "fr": { ...EN, ...FR },
        "de": { ...EN, ...DE },
        "ja": { ...EN, ...JA },
        "ko": { ...EN, ...KO },
        "ru": { ...EN, ...RU },
        "pt-BR": { ...EN, ...PTBR },
        "ar": { ...EN, ...AR },
        "hi": { ...EN, ...HI },
    };

    const SUPPORTED = Object.keys(TRANSLATIONS);

    function normalizeLang(raw) {
        if (!raw) return "en";
        const value = raw.toLowerCase();
        if (value.startsWith("zh")) return "zh-CN";
        if (value.startsWith("pt")) return "pt-BR";
        if (value.startsWith("es")) return "es";
        if (value.startsWith("fr")) return "fr";
        if (value.startsWith("de")) return "de";
        if (value.startsWith("ja")) return "ja";
        if (value.startsWith("ko")) return "ko";
        if (value.startsWith("ru")) return "ru";
        if (value.startsWith("ar")) return "ar";
        if (value.startsWith("hi")) return "hi";
        if (value.startsWith("en")) return "en";
        return "en";
    }

    function getPreferredLang() {
        const saved = localStorage.getItem("nola_lang");
        if (saved && SUPPORTED.includes(saved)) return saved;
        const browserLangs = navigator.languages && navigator.languages.length
            ? navigator.languages
            : [navigator.language || "en"];
        for (const lang of browserLangs) {
            const normalized = normalizeLang(lang);
            if (SUPPORTED.includes(normalized)) return normalized;
        }
        return "en";
    }

    function applyLocale(locale) {
        const lang = SUPPORTED.includes(locale) ? locale : "en";
        const dict = TRANSLATIONS[lang] || EN;

        document.querySelectorAll("[data-i18n]").forEach((el) => {
            const key = el.getAttribute("data-i18n");
            if (dict[key] != null) el.textContent = dict[key];
        });
        document.querySelectorAll("[data-i18n-html]").forEach((el) => {
            const key = el.getAttribute("data-i18n-html");
            if (dict[key] != null) el.innerHTML = dict[key];
        });

        document.title = dict.title || EN.title;
        const metaDesc = document.getElementById("metaDescription");
        const ogDesc = document.getElementById("metaOgDescription");
        if (metaDesc) metaDesc.setAttribute("content", dict.meta_description || EN.meta_description);
        if (ogDesc) ogDesc.setAttribute("content", dict.og_description || EN.og_description);

        document.documentElement.lang = lang;
        document.documentElement.dir = lang === "ar" ? "rtl" : "ltr";

        const selector = document.getElementById("langSelect");
        if (selector) selector.value = lang;
        localStorage.setItem("nola_lang", lang);
    }

    // Sticky header shadow on scroll
    const header = document.getElementById("siteHeader");
    const onScroll = () => {
        if (!header) return;
        if (window.scrollY > 8) header.classList.add("scrolled");
        else header.classList.remove("scrolled");
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    onScroll();

    // Language selector
    const langSelect = document.getElementById("langSelect");
    const initialLang = getPreferredLang();
    applyLocale(initialLang);
    if (langSelect) {
        langSelect.addEventListener("change", (e) => {
            applyLocale(normalizeLang(e.target.value));
        });
    }

    // Mobile menu toggle
    const menuBtn = document.getElementById("menuBtn");
    const mobileMenu = document.getElementById("mobileMenu");
    if (menuBtn && mobileMenu) {
        menuBtn.addEventListener("click", () => {
            const open = mobileMenu.classList.toggle("open");
            menuBtn.setAttribute("aria-expanded", open ? "true" : "false");
        });
        mobileMenu.addEventListener("click", (e) => {
            if (e.target.tagName === "A") {
                mobileMenu.classList.remove("open");
                menuBtn.setAttribute("aria-expanded", "false");
            }
        });
    }

    // Reveal-on-scroll
    const reveals = document.querySelectorAll(".reveal");
    if ("IntersectionObserver" in window) {
        const io = new IntersectionObserver((entries) => {
            entries.forEach((entry) => {
                if (entry.isIntersecting) {
                    entry.target.classList.add("in");
                    io.unobserve(entry.target);
                }
            });
        }, { threshold: 0.12, rootMargin: "0px 0px -40px 0px" });
        reveals.forEach((el) => io.observe(el));
    } else {
        reveals.forEach((el) => el.classList.add("in"));
    }

    // Animated counters
    const counters = document.querySelectorAll("[data-count]");
    const animateCount = (el) => {
        const target = parseInt(el.getAttribute("data-count"), 10);
        if (isNaN(target)) return;
        const duration = 1400;
        const start = performance.now();
        const tick = (now) => {
            const t = Math.min((now - start) / duration, 1);
            const eased = 1 - Math.pow(1 - t, 3);
            el.textContent = Math.round(target * eased).toString();
            if (t < 1) requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
    };
    if ("IntersectionObserver" in window) {
        const co = new IntersectionObserver((entries) => {
            entries.forEach((entry) => {
                if (entry.isIntersecting) {
                    animateCount(entry.target);
                    co.unobserve(entry.target);
                }
            });
        }, { threshold: 0.5 });
        counters.forEach((el) => co.observe(el));
    } else {
        counters.forEach((el) => { el.textContent = el.getAttribute("data-count"); });
    }
})();
