import type { Locale } from '../i18n';
const languages: Locale[] = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'];
const entries: Record<string, readonly string[]> = {
    configure: ['配置 {name} · 气泡颜色', 'Configure {name} · Bubble color', '{name} konfigurieren · Blasenfarbe', 'Configurar {name} · Color de burbuja', 'Configurer {name} · Couleur de bulle', '{name}の設定 · 吹き出しの色', 'Настроить {name} · Цвет сообщения'],
    bubble: ['气泡颜色', 'Bubble color', 'Blasenfarbe', 'Color de burbuja', 'Couleur de bulle', '吹き出しの色', 'Цвет сообщения'],
    preview: ['这是尚未保存的颜色预览。', 'This color preview is not saved yet.', 'Diese Farbvorschau ist noch nicht gespeichert.', 'Esta vista previa aún no está guardada.', 'Cet aperçu de couleur n’est pas encore enregistré.', 'この色のプレビューはまだ保存されていません。', 'Этот цвет ещё не сохранён.'],
    save: ['保存', 'Save', 'Speichern', 'Guardar', 'Enregistrer', '保存', 'Сохранить'],
    sessionOnly: ['浏览器未能保存偏好；颜色仅在本页有效。', 'The browser could not save this preference; the color applies only on this page.', 'Der Browser konnte die Einstellung nicht speichern; die Farbe gilt nur auf dieser Seite.', 'El navegador no pudo guardar la preferencia; el color solo se aplica en esta página.', 'Le navigateur n’a pas pu enregistrer la préférence ; la couleur s’applique uniquement sur cette page.', '設定を保存できませんでした。色はこのページでのみ有効です。', 'Браузер не сохранил настройку; цвет действует только на этой странице.'],
    red: ['红', 'Red', 'Rot', 'Rojo', 'Rouge', '赤', 'Красный'],
    orange: ['橙', 'Orange', 'Orange', 'Naranja', 'Orange', 'オレンジ', 'Оранжевый'],
    yellow: ['黄', 'Yellow', 'Gelb', 'Amarillo', 'Jaune', '黄', 'Жёлтый'],
    green: ['绿', 'Green', 'Grün', 'Verde', 'Vert', '緑', 'Зелёный'],
    teal: ['青', 'Teal', 'Türkis', 'Verde azulado', 'Sarcelle', '青緑', 'Бирюзовый'],
    blue: ['蓝', 'Blue', 'Blau', 'Azul', 'Bleu', '青', 'Синий'],
    purple: ['紫', 'Purple', 'Violett', 'Morado', 'Violet', '紫', 'Фиолетовый'],
    pink: ['粉', 'Pink', 'Rosa', 'Rosa', 'Rose', 'ピンク', 'Розовый'],
};
export const expertColorMessages: Record<Locale, Record<string, string>> = Object.fromEntries(languages.map((language, index) => [language,
    Object.fromEntries(Object.entries(entries).map(([key, values]) => [`expertColor.${key}`, values[index]])),
])) as Record<Locale, Record<string, string>>;
