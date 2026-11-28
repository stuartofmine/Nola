export type InspectorPreviewItem = {
    title: string;
    subtitle?: string;
    kind: 'image' | 'pdf' | 'text' | 'file';
    mimeType?: string;
    path?: string;
    imageUrl?: string;
    pdfUrl?: string;
    text?: string;
};
