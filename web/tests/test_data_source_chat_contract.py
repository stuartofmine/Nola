from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def test_chat_data_sources_expose_real_conversation_policy_controls() -> None:
    template = (ROOT / "web/app/templates/chat.html").read_text(encoding="utf-8")

    assert "context-data-source-policy" in template
    assert 'role="switch"' in template
    assert "data-next-enabled" in template
    assert "conversationManaged ?" in template
    assert "Boolean(policy && source?.resolution)" in template
    assert "/data-sources/${encodeURIComponent(sourceId)}" in template
    assert "JSON.stringify({ enabled:" in template
    assert "renderChatDataSources(data.dataSources || [])" in template


def test_data_source_validation_has_visible_connect_inspect_save_progress() -> None:
    template = (ROOT / "web/app/templates/data_sources.html").read_text(encoding="utf-8")
    chat_template = (ROOT / "web/app/templates/chat.html").read_text(encoding="utf-8")
    styles = (ROOT / "web/app/static/css/style.css").read_text(encoding="utf-8")

    assert 'id="dataSourceValidationProgress"' in template
    assert 'data-validation-step="connect"' in template
    assert 'data-validation-step="inspect"' in template
    assert 'data-validation-step="save"' in template
    assert "beginDataSourceValidationProgress" in template
    assert ".data-source-validation-progress" in styles
    assert ".context-data-source-switch.is-on" in chat_template
