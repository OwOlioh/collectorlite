export function splitTagNames(input: string): string[] {
  return input
    .split(/[,，\n]/)
    .map((name) => name.trim())
    .filter(Boolean);
}
