import type { ReactNode } from "react";
import { api } from "./api";

// Only match http(s) URLs — never javascript:, data:, etc.
const URL_RE = /https?:\/\/[^\s]+/gi;

// Strip trailing punctuation that commonly sits next to a URL in prose.
const TRAILING_PUNCT = /[.,;:!?。，；：！？、）)\]}'"」』]+$/;

function stripTrailingPunct(url: string): string {
  return url.replace(TRAILING_PUNCT, "");
}

interface LinkifiedTextProps {
  text: string;
}

// Renders note text with http(s) links as clickable anchors that open in the
// system browser via the existing open_url command. Text is tokenized into
// React nodes (never dangerouslySetInnerHTML), so there is no injection risk.
export function LinkifiedText({ text }: LinkifiedTextProps) {
  if (!text) return null;

  const nodes: ReactNode[] = [];
  let last = 0;
  let key = 0;
  let match: RegExpExecArray | null;

  URL_RE.lastIndex = 0;
  while ((match = URL_RE.exec(text)) !== null) {
    const raw = match[0];
    const url = stripTrailingPunct(raw);
    const start = match.index;

    if (start > last) {
      nodes.push(text.slice(last, start));
    }
    nodes.push(
      <a
        key={key++}
        href={url}
        className="note-link"
        onClick={(event) => {
          event.preventDefault();
          void api.openUrl(url);
        }}
      >
        {url}
      </a>
    );
    last = start + raw.length;
  }
  if (last < text.length) {
    nodes.push(text.slice(last));
  }

  return <>{nodes}</>;
}
