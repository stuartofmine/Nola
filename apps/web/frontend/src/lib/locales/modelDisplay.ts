import type { Locale } from '../i18n';

const keys = ['topP', 'topK', 'minP', 'flashAttention', 'effort.none', 'effort.minimal', 'effort.low', 'effort.medium', 'effort.high', 'effort.xhigh', 'effort.max', 'localTimezone'];
const labels: Record<Locale, string[]> = {
    'zh-Hans': ['核采样（Top P）', '候选数量（Top K）', '最低概率（Min P）', '闪电注意力（Flash Attention）', '不启用', '极低', '低', '中', '高', '极高', '最高', '本地时区'],
    en: ['Top P', 'Top K', 'Min P', 'Flash Attention', 'None', 'Minimal', 'Low', 'Medium', 'High', 'Extra high', 'Maximum', 'Local time'],
    de: ['Top P', 'Top K', 'Min P', 'Flash Attention', 'Keine', 'Minimal', 'Niedrig', 'Mittel', 'Hoch', 'Sehr hoch', 'Maximum', 'Ortszeit'],
    es: ['Top P', 'Top K', 'Min P', 'Flash Attention', 'Ninguno', 'Mínimo', 'Bajo', 'Medio', 'Alto', 'Muy alto', 'Máximo', 'Hora local'],
    fr: ['Top P', 'Top K', 'Min P', 'Flash Attention', 'Aucun', 'Minimal', 'Faible', 'Moyen', 'Élevé', 'Très élevé', 'Maximum', 'Heure locale'],
    ja: ['累積確率（Top P）', '候補数（Top K）', '最小確率（Min P）', 'フラッシュアテンション（Flash Attention）', 'なし', '最小', '低', '中', '高', '非常に高', '最大', '現地時間'],
    ru: ['Top P', 'Top K', 'Min P', 'Flash Attention', 'Нет', 'Минимальные', 'Низкие', 'Средние', 'Высокие', 'Очень высокие', 'Максимальные', 'Местное время'],
};
export const modelDisplayMessages = Object.fromEntries(Object.entries(labels).map(([language, values]) => {
    if (values.length !== keys.length) throw new Error(`Incomplete display labels: ${language}`);
    return [language, Object.fromEntries(keys.map((key, index) => [`modelDisplay.${key}`, values[index]]))];
})) as Record<Locale, Record<string, string>>;
