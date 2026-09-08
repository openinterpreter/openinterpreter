<!-- README translation source: README.md sha256=d5db2b994d859b4186348f83bf35783a86b3f79e363d0c92100dbf06707ced16 -->

<h1 align="center">Open Interpreter</h1>

<p align="center">Un agente de programación optimizado para modelos de bajo costo. <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_text"><strong>Artículo del blog ↗</strong></a></p>

<p align="center">
  <a href="README.md">English</a> • <b>Español</b> • <a href="README_ZH.md">简体中文</a> • <a href="README_JA.md">日本語</a>
</p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=docs_badge"><img alt="Documentación" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="Licencia" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

> [!NOTE]
> **Hoy: Kimi K3 ya está aquí.** Hemos reimplementado en Rust el harness
> [Kimi Code](https://www.kimi.com/coding/en) recomendado por el proveedor,
> para ofrecer el máximo rendimiento de K3 con una interfaz similar a Codex.
> [**Documentación de Kimi →**](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_note)

<br>

<p align="center">
  <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_image">
    <img alt="Open Interpreter ejecutándose en una terminal" src="docs-site/assets/open-interpreter-terminal-hero.png" width="100%" />
  </a>
</p>

## Instalación

macOS y Linux:

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
```

Windows:

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

Después, escribe `i` o `interpreter` en tu terminal para iniciar una sesión.

## Emulación de harnesses

Open Interpreter es un fork de Codex de OpenAI enfocado en emular el harness de agente que ofrece el mejor rendimiento con modelos de bajo costo.

Usa `/harness` para cambiar el harness activo:

```text
> /harness

native
claude-code
claude-code-bare
zcode
kimi-code
kimi-cli
qwen-code
deepseek-tui
swe-agent
minimal
```

Consulta la [documentación de harnesses](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harness_docs) y las [guías de configuración de proveedores](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides).

## Compatible con ACP y Codex

Open Interpreter funciona en [editores y clientes compatibles con ACP](https://agentclientprotocol.com/get-started/clients). Configura el cliente para ejecutar `interpreter acp`; consulta la [guía de ACP](https://www.openinterpreter.com/docs/terminal/acp) para ver ejemplos.

¿Ya estás desarrollando con el SDK de Codex de OpenAI? Conserva el SDK y cambia una sola línea para usar otro binario:

```diff
-const codex = new Codex();
+const codex = new Codex({ codexPathOverride: "interpreter" });
```

Open Interpreter utiliza el mismo protocolo `exec` que Codex. Consulta la [guía del SDK](https://www.openinterpreter.com/docs/terminal/sdk) y ejecuta `scripts/test-codex-sdk-compat.sh` para realizar una comprobación local de compatibilidad que no requiere un proveedor.

## Uso de la computadora

Open Interpreter incluye una habilidad de QA que permite a cualquier modelo operar y probar interfaces. Puede controlar aplicaciones web en un navegador real con [agent-browser](https://github.com/vercel-labs/agent-browser), o manejar y probar aplicaciones nativas con [trycua](https://github.com/trycua/cua).

## Características

- Ejecuta comandos con aislamiento nativo en macOS, Linux y Windows.
- Cambia de proveedor y modelo desde la TUI con `/model`.
- Ejecuta cualquier proveedor compatible con OpenAI mediante Chat Completions
  con `interpreter --chat-completions` o `interpreter exec --chat-completions`.
- Inspecciona o cambia harnesses de modelos nativos de Rust con `/harness`.
- Prueba aplicaciones web y nativas mediante la habilidad de QA integrada.
- Funciona como agente del [Protocolo de Cliente de Agente](https://agentclientprotocol.com/) para editores mediante `interpreter acp`.
- Mantiene la configuración y el estado de las sesiones en `~/.openinterpreter`.
- Es compatible con `exec`, MCP, habilidades, hooks, permisos y `AGENTS.md`.

## Documentación

- [Documentación de la terminal](https://www.openinterpreter.com/docs/terminal?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=terminal_docs)
- [Inicio rápido](https://www.openinterpreter.com/docs/terminal/quickstart?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=quickstart)
- [Guía de instalación](https://www.openinterpreter.com/docs/terminal/install?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=install_guide)
- [Configuración](https://www.openinterpreter.com/docs/terminal/config?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=configuration)
- [Referencia de la CLI](https://www.openinterpreter.com/docs/terminal/cli-reference?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=cli_reference)
- [Harnesses](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harnesses)
- [Guías de proveedores de modelos](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides)
  - [Kimi K3](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_docs)
  - [DeepSeek](https://www.openinterpreter.com/docs/terminal/deepseek?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=deepseek_docs)
  - [Z.AI, GLM y ZCode](https://www.openinterpreter.com/docs/terminal/zai-glm?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=zai_glm_docs)
- [Protocolo de Cliente de Agente](https://www.openinterpreter.com/docs/terminal/acp)
- [SDK de Codex](https://www.openinterpreter.com/docs/terminal/sdk)
- [Aislamiento y aprobaciones](https://www.openinterpreter.com/docs/terminal/sandbox?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=sandbox_approvals)
- [Personalizar la marca de una distribución derivada](FORK_BRANDING.md)

La lista de proveedores y modelos se genera automáticamente; no se mantiene como listas en Rust. Desde `codex-rs`, actualiza todos los proveedores alojados con `python3 scripts/write_provider_catalog.py`, o repite `--provider <provider-id>` para actualizar solo los proveedores seleccionados. Las fuentes de modelos en vivo requieren las credenciales del proveedor que se indican en la [documentación de proveedores](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_catalog_generation).

> [!NOTE]
> Esta es la nueva versión de Open Interpreter en Rust, basada en Codex. ¿Buscas el proyecto original en Python? Continúa como un fork mantenido por la comunidad en [endolith/open-interpreter](https://github.com/endolith/open-interpreter).

## Licencia

Apache-2.0
