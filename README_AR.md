<!-- README translation source: README.md sha256=890ad2844696117f8baf7b853fa308a1a56812104ccc7d72678531680114b546 -->

<h1 align="center">Open Interpreter</h1>

<div dir="rtl">

<p align="center">وكيل برمجة مُحسَّن للنماذج منخفضة التكلفة. <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_text"><strong>مقالة المدونة ↗</strong></a></p>

</div>

<p align="center">
  <a href="README.md">English</a> • <a href="README_ES.md">Español</a> • <a href="README_ZH.md">简体中文</a> • <a href="README_JA.md">日本語</a> • <b>العربية</b>
</p>

<p align="center">
  <a href="https://discord.gg/Hvz9Axh84z"><img alt="Discord" src="https://img.shields.io/discord/1146610656779440188?style=flat-square&label=Discord" /></a>
  <a href="https://www.openinterpreter.com/docs/terminal?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=docs_badge"><img alt="التوثيق" src="https://img.shields.io/badge/Documentation-white?style=flat-square" /></a>
  <a href="LICENSE"><img alt="الرخصة" src="https://img.shields.io/badge/License-Apache--2.0-white?style=flat-square" /></a>
</p>

> [!NOTE]
> **اليوم: وصل Kimi K3.** أعدنا تنفيذ الـ harness الذي يوصي به المزوّد،
> [Kimi Code](https://www.kimi.com/coding/en)، بلغة Rust، ليمنحك أقصى أداء
> لنموذج K3 بواجهة شبيهة بواجهة Codex.
> [**توثيق Kimi →**](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_note)

<br>

<p align="center">
  <a href="https://www.openinterpreter.com/blog/open-interpreter?utm_source=github&amp;utm_medium=referral&amp;utm_campaign=readme&amp;utm_content=hero_image">
    <img alt="Open Interpreter يعمل داخل الطرفية" src="docs-site/assets/open-interpreter-terminal-hero.png" width="100%" />
  </a>
</p>

<div dir="rtl">

## التثبيت

على macOS وLinux:

</div>

```bash
curl -fsSL https://www.openinterpreter.com/install | sh
```

<div dir="rtl">

على Windows:

</div>

```powershell
irm https://www.openinterpreter.com/install.ps1 | iex
```

<div dir="rtl">

بعد ذلك اكتب `i` أو `interpreter` في الطرفية لبدء جلسة.

## محاكاة الـ Harness

Open Interpreter هو فورك (fork) من Codex التابع لـ OpenAI، ويركّز على محاكاة الـ harness الخاص بالوكيل، أي الإطار الذي يستخرج أفضل أداء من النماذج منخفضة التكلفة.

استخدم `/harness` لتبديل الـ harness النشط:

</div>

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

<div dir="rtl">

اقرأ المزيد في [توثيق الـ harness](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harness_docs) و[أدلة إعداد المزوّدين](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides).

## متوافق مع ACP ومتوافق مع Codex

يعمل Open Interpreter في [المحررات والعملاء المتوافقة مع ACP](https://agentclientprotocol.com/get-started/clients). اضبط العميل ليشغّل `interpreter acp`؛ وتجد أمثلة في [دليل ACP](https://www.openinterpreter.com/docs/terminal/acp).

هل تبني بالفعل على Codex SDK من OpenAI؟ أبقِ الـ SDK كما هو
واستبدل مسار الملف التنفيذي بسطر واحد:

</div>

```diff
-const codex = new Codex();
+const codex = new Codex({ codexPathOverride: "interpreter" });
```

<div dir="rtl">

يتحدث Open Interpreter بروتوكول exec نفسه الذي يستخدمه Codex. راجع [دليل SDK](https://www.openinterpreter.com/docs/terminal/sdk) وشغّل `scripts/test-codex-sdk-compat.sh` لإجراء فحص توافق محلي لا يحتاج إلى مزوّد.

هل تريد واجهة متصفح صغيرة فوق Open Interpreter الخام؟ ابدأ بـ
[دليل دردشة الويب المحلية بالبث](docs/streaming-web-chat.md). وللحصول على
عميل كامل لسطح المكتب أو المتصفح أو دون واجهة (headless)، استخدم
[Interpreter Workstation](https://github.com/openinterpreter/interpreter-workstation).

## قابل للنقل افتراضيًا

ينبغي أن يندمج Open Interpreter في إعداد الوكلاء الذي تعتمده أصلًا، بدلًا من
حبسه في صيغة خاصة بـ Open Interpreter وحده. وهدف المنتج هو تفضيل المعايير
والمجلدات المشتركة المحايدة للأدوات، والإبقاء على البيانات التي يكتبها
المستخدم في ملفات مقروءة، وجعل الانتقال إلى وكيل متوافق آخر أو منه أمرًا
سهلًا.

ويشمل ذلك اليوم ملف `AGENTS.md` في المستودع، ومجلدات `.agents/skills`
المشتركة، وMCP، وACP، وبروتوكول exec الخاص بـ Codex. أما التخزين الخاص بالمنتج
تحت `~/.openinterpreter` فهو مخصص للإعدادات وحالة التشغيل التي لا يتوفر لها
بعد معيار مشترك عملي. وتبقى مجلدات المهارات القديمة الخاصة بالمنتج قابلة
للقراءة حفاظًا على التوافق، لكن المهارات الجديدة مكانها `.agents/skills` أو
`~/.agents/skills`.

راجع [دليل قابلية النقل](docs/portability.md) لمعرفة الحد الحالي وقواعد
تطويره.

## استخدام الحاسوب

يأتي Open Interpreter مع مهارة QA تتيح لأي نموذج تشغيل الواجهات واختبارها. يمكنه تشغيل تطبيقات الويب في متصفح حقيقي عبر [agent-browser](https://github.com/vercel-labs/agent-browser)، أو تشغيل التطبيقات الأصلية واختبارها عبر [trycua](https://github.com/trycua/cua).

## المزايا

- يشغّل الأوامر داخل عزل أصلي (sandboxing) على macOS وLinux وWindows.
- يبدّل المزوّدين والنماذج من واجهة الطرفية التفاعلية (TUI) عبر `/model`.
- يشغّل أي مزوّد متوافق مع OpenAI تختاره عبر Chat Completions باستخدام
  `interpreter --chat-completions` أو `interpreter exec --chat-completions`.
- يعرض الـ harnesses الخاصة بالنماذج والمنفَّذة أصلًا بلغة Rust أو يبدّل بينها عبر `/harness`.
- يختبر تطبيقات الويب والتطبيقات الأصلية عبر مهارة QA المدمجة.
- يعمل وكيلًا وفق [Agent Client Protocol](https://agentclientprotocol.com/) للمحررات عبر `interpreter acp`.
- يعيد استخدام تعليمات `AGENTS.md` المشتركة ومجلدات `.agents/skills`.
- يحتفظ بإعدادات المنتج وحالة الجلسات محليًا تحت `~/.openinterpreter`.
- يدعم `exec` وMCP والمهارات والـ hooks والأذونات و`AGENTS.md`.

## التوثيق

- [توثيق الطرفية](https://www.openinterpreter.com/docs/terminal?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=terminal_docs)
- [البدء السريع](https://www.openinterpreter.com/docs/terminal/quickstart?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=quickstart)
- [دليل التثبيت](https://www.openinterpreter.com/docs/terminal/install?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=install_guide)
- [الإعدادات](https://www.openinterpreter.com/docs/terminal/config?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=configuration)
- [مرجع واجهة سطر الأوامر (CLI)](https://www.openinterpreter.com/docs/terminal/cli-reference?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=cli_reference)
- [الـ Harnesses](https://www.openinterpreter.com/docs/terminal/harness?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=harnesses)
- [أدلة مزوّدي النماذج](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_guides)
  - [Kimi K3](https://www.openinterpreter.com/docs/terminal/kimi-k3?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=kimi_k3_docs)
  - [DeepSeek](https://www.openinterpreter.com/docs/terminal/deepseek?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=deepseek_docs)
  - [Z.AI وGLM وZCode](https://www.openinterpreter.com/docs/terminal/zai-glm?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=zai_glm_docs)
- [Agent Client Protocol](https://www.openinterpreter.com/docs/terminal/acp)
- [Codex SDK](https://www.openinterpreter.com/docs/terminal/sdk)
- [قابلية النقل](https://github.com/openinterpreter/openinterpreter/blob/main/docs/portability.md)
- [العزل (Sandbox) والموافقات](https://www.openinterpreter.com/docs/terminal/sandbox?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=sandbox_approvals)
- [تخصيص العلامة التجارية لنسخة متفرعة للتوزيع](FORK_BRANDING.md)

قوائم المزوّدين والنماذج تُولَّد توليدًا ولا تُدار يدويًا كقوائم Rust. من داخل
`codex-rs`، حدّث جميع المزوّدين المستضافين بتشغيل
`python3 scripts/write_provider_catalog.py`، أو كرّر الخيار
`--provider <provider-id>` لتحديث مدخلات مزوّدين محدّدين فقط. تتطلب مصادر
النماذج الحية بيانات اعتماد المزوّد الموضحة في
[توثيق المزوّدين](https://www.openinterpreter.com/docs/terminal/providers?utm_source=github&utm_medium=referral&utm_campaign=readme&utm_content=provider_catalog_generation).

</div>


> [!NOTE]
> هذه هي النسخة الجديدة من Open Interpreter المكتوبة بلغة Rust والمبنية على Codex. هل تبحث عن مشروع Python الأصلي؟ ما زال قائمًا كنسخة متفرعة (fork) يتولى المجتمع صيانتها على [endolith/open-interpreter](https://github.com/endolith/open-interpreter).

<div dir="rtl">

## الرخصة

Apache-2.0

</div>

ترجمة: فريق مُشَيِّد
