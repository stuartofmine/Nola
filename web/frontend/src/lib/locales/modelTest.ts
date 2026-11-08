import type { Locale } from '../i18n';

const languages: readonly Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
type Translations = readonly [string, string, string, string, string, string, string];
const entries: Record<string, Translations> = {
    'modelTest.title': ['测试模型', 'Test Model', 'Modell testen', 'Probar modelo', 'Tester le modèle', 'モデルをテスト', 'Проверить модель'],
    'modelTest.subtitle': ['使用已保存的模型配置运行测试，并查看输出和耗时。', 'Run a test with the saved model configuration and inspect its output and timing.', 'Mit der gespeicherten Modellkonfiguration testen und Ausgabe sowie Zeiten prüfen.', 'Ejecuta una prueba con la configuración guardada y revisa la salida y los tiempos.', 'Exécutez un test avec la configuration enregistrée et consultez la sortie et les durées.', '保存済みのモデル設定でテストし、出力と所要時間を確認します。', 'Проверьте модель с сохранённой конфигурацией и оцените ответ и время работы.'],
    'modelTest.back': ['返回模型', 'Back to Models', 'Zurück zu Modellen', 'Volver a modelos', 'Retour aux modèles', 'モデルに戻る', 'К моделям'],
    'modelTest.parameters': ['参数…', 'Parameters…', 'Parameter…', 'Parámetros…', 'Paramètres…', 'パラメータ…', 'Параметры…'],
    'modelTest.pane': ['模型测试区域', 'Model test section', 'Modelltestbereich', 'Sección de prueba del modelo', 'Section de test du modèle', 'モデルテストのセクション', 'Раздел проверки модели'],
    'modelTest.setup': ['设置', 'Setup', 'Einrichtung', 'Configuración', 'Configuration', '設定', 'Настройка'],
    'modelTest.output': ['输出', 'Output', 'Ausgabe', 'Salida', 'Sortie', '出力', 'Ответ'],
    'modelTest.defaultPrompt': ['请用三个简洁的要点说明 Nola 可以做什么。', 'Explain what Nola can do in three concise bullet points.', 'Erkläre in drei knappen Stichpunkten, was Nola kann.', 'Explica en tres puntos breves qué puede hacer Nola.', 'Explique en trois points concis ce que Nola peut faire.', 'Nola でできることを簡潔な箇条書き三つで説明してください。', 'Кратко объясни в трёх пунктах, что умеет Nola.'],
    'modelTest.prompt': ['测试提示词', 'Test prompt', 'Test-Prompt', 'Instrucción de prueba', 'Instruction de test', 'テスト用プロンプト', 'Тестовый запрос'],
    'modelTest.notConfigured': ['尚未配置可用模型', 'No model configured', 'Kein Modell konfiguriert', 'No hay un modelo configurado', 'Aucun modèle configuré', '利用可能なモデルがありません', 'Модель не настроена'],
    'modelTest.safeHint': ['测试使用安全权限，不会保留测试对话；中断时可到对话页恢复。', 'Tests use safe permissions. Completed test conversations are removed; interrupted runs can be recovered in Chat.', 'Tests verwenden sichere Berechtigungen. Abgeschlossene Testdialoge werden entfernt; unterbrochene Läufe können im Chat fortgesetzt werden.', 'Las pruebas usan permisos seguros. Se eliminan las conversaciones completadas; las interrumpidas pueden recuperarse en Chat.', 'Les tests utilisent des permissions sûres. Les conversations terminées sont supprimées ; les exécutions interrompues peuvent être reprises dans Chat.', 'テストは安全な権限で実行されます。完了したテスト会話は削除され、中断した実行はチャットで再開できます。', 'Тесты используют безопасные разрешения. Завершённые тестовые диалоги удаляются; прерванные можно восстановить в чате.'],
    'modelTest.metrics': ['指标', 'Metrics', 'Messwerte', 'Métricas', 'Mesures', '指標', 'Показатели'],
    'modelTest.firstToken': ['首字延迟', 'First token', 'Erstes Token', 'Primer token', 'Premier jeton', '最初のトークン', 'Первый токен'],
    'modelTest.totalTime': ['总耗时', 'Total time', 'Gesamtdauer', 'Tiempo total', 'Durée totale', '合計時間', 'Общее время'],
    'modelTest.characters': ['字符数', 'Characters', 'Zeichen', 'Caracteres', 'Caractères', '文字数', 'Символы'],
    'modelTest.run': ['运行', 'Run', 'Starten', 'Ejecutar', 'Exécuter', '実行', 'Запустить'],
    'modelTest.stop': ['停止', 'Stop', 'Stoppen', 'Detener', 'Arrêter', '停止', 'Остановить'],
    'modelTest.resume': ['恢复', 'Resume', 'Fortsetzen', 'Reanudar', 'Reprendre', '再開', 'Продолжить'],
    'modelTest.clear': ['清空', 'Clear', 'Leeren', 'Borrar', 'Effacer', 'クリア', 'Очистить'],
    'modelTest.running': ['正在运行测试…', 'Running test…', 'Test läuft…', 'Ejecutando prueba…', 'Test en cours…', 'テストを実行中…', 'Идёт проверка…'],
    'modelTest.empty': ['运行模型测试后，输出将在这里显示。', 'Run a model test to inspect its response here.', 'Starten Sie einen Modelltest, um die Antwort hier zu sehen.', 'Ejecuta una prueba para ver aquí la respuesta.', 'Lancez un test pour voir la réponse ici.', 'モデルテストを実行すると、ここに応答が表示されます。', 'Запустите проверку модели, чтобы увидеть ответ здесь.'],
    'modelTest.failed': ['模型测试失败。', 'Model test failed.', 'Modelltest fehlgeschlagen.', 'La prueba del modelo falló.', 'Échec du test du modèle.', 'モデルテストに失敗しました。', 'Проверка модели завершилась ошибкой.'],
    'modelTest.interrupted': ['连接中断；先恢复本次运行，不要重复提交。', 'Connection interrupted. Resume this run before starting another.', 'Verbindung unterbrochen. Setzen Sie diesen Lauf fort, bevor Sie einen neuen starten.', 'Conexión interrumpida. Reanuda esta ejecución antes de iniciar otra.', 'Connexion interrompue. Reprenez cette exécution avant d’en lancer une autre.', '接続が中断しました。新しいテストを始める前にこの実行を再開してください。', 'Соединение прервано. Продолжите этот запуск, прежде чем начинать новый.'],
    'modelTest.cleanupFailed': ['测试已结束，但未能删除临时会话。可在对话页查看并手动处理。', 'The test finished, but its temporary conversation could not be removed. Review it in Chat.', 'Der Test ist beendet, aber die temporäre Unterhaltung konnte nicht entfernt werden. Prüfen Sie sie im Chat.', 'La prueba terminó, pero no se pudo eliminar la conversación temporal. Revísala en Chat.', 'Le test est terminé, mais la conversation temporaire n’a pas pu être supprimée. Consultez-la dans Chat.', 'テストは完了しましたが、一時会話を削除できませんでした。チャットで確認してください。', 'Тест завершён, но временный диалог не удалось удалить. Проверьте его в чате.'],
    'modelTest.openConversation': ['打开会话', 'Open conversation', 'Unterhaltung öffnen', 'Abrir conversación', 'Ouvrir la conversation', '会話を開く', 'Открыть диалог'],
};

export const modelTestMessages = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [key, values[index]])),
])) as Record<Locale, Record<string, string>>;
