Array.from(document.querySelectorAll('article[data-id]'),el=>({id:el.getAttribute('data-id'),price:Number(el.getAttribute('data-price'))}))
