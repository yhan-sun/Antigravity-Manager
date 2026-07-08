# 关于 view_image 导致 Base64 乱码与 Token 暴涨的原因分析报告

您提供的两个日志文件（`original_request.json` 和 `v1internal_request.json`）非常关键，它彻底揭示了为什么仅仅调用了一个 `view_image` 函数，就会导致几十万 Token 的乱码灾难。

## 1. 事情的起因：官方 Antigravity 客户端的特殊机制
当智能体（例如 Codex）调用 `view_image(path="...")` 工具去查看本地图片时，实际发生的过程如下：
1. 模型输出 `function_call`，要求调用 `view_image`。
2. **官方的 Desktop 客户端**（即 Antigravity 桌面端）捕获到这个工具调用，会在本地界面上帮您把这张图**渲染显示**出来。

## 2. 灾难的发生：历史记录的“强制序列化”
当您在界面上看完图片，继续发送下一条消息（触发新一轮对话）时，官方客户端需要把**之前所有的聊天记录**重新打包发送给代理（也就是打成 `original_request.json` 发出来）。

这里官方客户端采用了一个非常“偷懒”但也符合前端逻辑的做法：
- 它没有把刚才那张图片作为独立的文件或原生的 `Blob/inlineData` 对象发送。
- 相反，它把图片在前端直接转换成了 **Base64 编码**，并硬生生塞进了一段 Markdown 语法的纯文本里：
  ```markdown
  ![image](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB...)
  ```
- 从您给的日志中可以看到，这段极其庞大的字符串，被放在了 `{"type": "input_text", "text": "..."}` 这样的文本块中发送给了您的反代（Antigravity-Manager2）。

## 3. 反代网关（修改前）的无心之失
由于官方客户端发来的格式里，明确写着 `"type": "input_text"`（我只是一段文本），所以您的反代网关（也就是我们修改前的代码）毫不犹豫地相信了它。

修改前的代码遇到 `input_text` 时，直接把它原封不动地放进了发给 Gemini 的 `{"text": "..."}` 字段中。

**结果就是：**
Gemini 收到了一段长达数万字的、由大小写字母组成的 ASCII 乱码（它并不知道这是一个 Markdown 图片，只觉得是一堆废话字母）。这不仅瞬间榨干了上下文 Token，还经常导致大模型产生幻觉，以为程序出错了。

## 4. 为什么我们的修复是完美的？
我们刚才在代码里写的那个 `parse_markdown_images_to_parts` 正则拦截器，就是专门对付官方客户端这个毛病的！

现在，当您的反代再次收到含有 `![image](data:image/...;base64,...)` 的 `input_text` 文本时：
1. 我们的拦截器会像安检机一样扫描出这段隐藏的 Base64 代码。
2. 把它从 `text` 字段里抠出来。
3. 把抠出来的部分，重组为 Gemini 官方支持的极低成本原生图像格式：
   ```json
   {
     "inlineData": {
       "mimeType": "image/png",
       "data": "iVBORw0KGgo..."
     }
   }
   ```
4. 原本那些几万字的无意义乱码，瞬间变成了清晰的机器视觉输入。

## 总结
**罪魁祸首**是官方 Desktop 客户端为了在历史记录中方便存储界面状态，强行将本地图片转换成了带有 Base64 的 Markdown 文本并伪装成普通文字发送。
而**我们的反代拦截器**则完美地充当了“翻译官”，在它发往大模型前将其还原为了原生图像。
