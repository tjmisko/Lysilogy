import { findRegexMatches } from "./regexSearch";
self.onmessage = (event: MessageEvent<{ text: string; pattern: string }>) => {
  self.postMessage(findRegexMatches(event.data.text, event.data.pattern));
};
