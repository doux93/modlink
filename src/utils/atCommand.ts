export function isAtCommand(text: string): boolean {
  return /^AT(?:[+^&*]|[A-Z]|\s|$)/i.test(text.trim());
}
