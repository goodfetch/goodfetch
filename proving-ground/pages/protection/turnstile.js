// The Turnstile page's widget, as Turnstile's script leaves it in a page:
// a hidden field for the token inside the widget's element, and the page's
// callback, which takes the token and submits the form.
(() => {
  "use strict";
  const widget = document.querySelector(".cf-turnstile");
  const field = document.createElement("input");
  field.type = "hidden";
  field.name = widget.getAttribute("data-response-field-name");
  widget.append(field);
  window[widget.getAttribute("data-callback")] = (token) => {
    field.value = token;
    widget.closest("form").requestSubmit();
  };
})();
