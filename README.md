# Supertonic OpenAI TTS Server

An OpenAI-compatible Text-to-Speech (TTS) server powered by **Supertonic 2**, a lightning-fast, on-device TTS system. This server mimics the OpenAI `v1/audio/speech` endpoint, allowing you to use Supertonic with existing OpenAI client libraries.

> **Attribution & Thanks**
>
> This project is built upon the incredible [Supertonic 2](https://huggingface.co/Supertone/supertonic-2) model developed by **Supertone Inc.**

## Usage

### Cloning the repo and dependencies

```bash
git clone https://github.com/sameert89/supertonic-tts-openai.git

cd supertonic-tts-openai

git clone git clone https://huggingface.co/Supertone/supertonic-2 assets
```

### 1. Run with Docker (Recommended)

```bash
docker run -p 8080:8080 -v $(pwd)/cache:/app/cache ghcr.io/sameert89/supertonic-tts-openai:latest
```

### 2. Run Directly 

Ensure you have Rust and ffmpeg installed.

```bash
cargo run --release --bin server
```

The server listens on port `8080`.

## API Reference

### Endpoint: `POST /v1/audio/speech`

Generates audio from the input text.

**Headers:**
- `Content-Type: application/json`
- `Authorization`: (Ignored, but accepted for compatibility)

**JSON Body Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `model` | string | No | The model name. Defaults to `supertonic-2`. `tts-1` is also accepted. |
| `input` | string | **Yes** | The text to generate audio for. Use `|` to separate segments for multi-language generation. |
| `voice` | string | **Yes** | The voice name (e.g., "Alex", "Sarah"). See **Available Voices** below. |
| `response_format` | string | No | Audio format: `mp3` (default), `opus`, `aac`, `flac`, `wav`, `pcm`. |
| `speed` | float | No | The speed of the generated audio. 0.25 to 4.0. Default `1.0`. |
| `total_step` | integer | No | **(Supertonic Extension)** Quality of generation (1-10). Default `5`. Higher is better but slower. |
| `lang` | string | No | **(Supertonic Extension)** Language code(s). Default `en`. See **Multilingual Support** below. |

### Available Voices

| Name | Gender | ID |
|------|--------|----|
| **Alex** | Male | M1 |
| **James** | Male | M2 |
| **Robert** | Male | M3 |
| **Sam** | Male | M4 |
| **Daniel** | Male | M5 |
| **Sarah** | Female | F1 |
| **Lily** | Female | F2 |
| **Jessica** | Female | F3 |
| **Olivia** | Female | F4 |
| **Emily** | Female | F5 |

*Note: For compatibility with existing tools, standard OpenAI voice names (like `alloy`, `echo`) are mapped to the closest equivalent in this list.*

### Multilingual Support

You can generate speech in multiple languages within a single request by splitting your `input` with pipes (`|`) and providing a comma-separated list of languages in `lang`.

**Supported Languages:**
- `en` (English)
- `ko` (Korean)
- `es` (Spanish)
- `pt` (Portuguese)
- `fr` (French)

**Example: Switching Languages**

To speak the first part in English and the second in French:

```bash
curl http://localhost:8080/v1/audio/speech \
  -H "Content-Type: application/json" \
  -d '{
    "model": "supertonic-2",
    "input": "Hello my friend | Bonjour mon ami",
    "voice": "Sarah",
    "lang": "en,fr",
    "total_step": 10
  }' \
  --output multi_lang.mp3
```

**Example: Standard Request**

```bash
curl http://localhost:8080/v1/audio/speech \
  -H "Content-Type: application/json" \
  -d '{
    "model": "supertonic-2",
    "input": "The quick brown fox jumps over the lazy dog.",
    "voice": "Alex",
    "speed": 1.0
  }' \
  --output speech.mp3
```

## License

The server code is open source. The Supertonic models used by this server are subject to their own license terms provided by Supertone Inc.
