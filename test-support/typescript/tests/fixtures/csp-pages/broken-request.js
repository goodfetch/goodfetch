// Fetches a same-origin URL whose connection the server drops.
fetch("/broken").catch(() => {});
