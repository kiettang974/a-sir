# ASR Studio — Product Context

## Product
ASR Studio is a macOS desktop application for offline Vietnamese speech-to-text transcription and AI-assisted meeting minutes. Audio decoding and speech recognition run entirely on-device (Whisper Large V3 Turbo / Large V3 Full / PhoWhisper via `whisper.cpp` + Metal, and Qwen3-ASR-1.7B via pure Rust `qwen-asr` + Silero VAD). Only when the user explicitly requests a summary does the app send the transcript text, glossary, and template columns to their chosen AI provider (Claude, ChatGPT, or Gemini).

## Primary User & Usage Scene
- **User:** Vietnamese business analyst, project manager, or team lead reviewing recorded Vietnamese/bilingual meetings (30–120 minutes) at a desktop workstation during daytime or office lighting.
- **Core Jobs:**
  1. Queue audio files or Voice Memos recordings and transcribe offline with resumable checkpoints.
  2. Read, verify, and clean transcripts (applying custom domain glossary terms).
  3. Generate, edit, verify (via click-to-seek timestamp audio playback), and export meeting minutes to Email HTML, Excel (`.xlsx`), or Word (`.docx`) using built-in or custom templates.

## Surface Mode
- **Primary mode:** `Operate` + `Read`.
- The interface stays calm and recedes behind the transcript and meeting-note document. Generous whitespace, clear action hierarchy (one primary action per toolbar, secondary actions tucked into a compact `⋯` overflow menu), and trustworthy native-feeling states outweigh decorative novelty.
