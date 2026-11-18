// pages-version: 1
// requires-app: 1.0.0
// Grand Theft Auto VI: the game's own pages the app translates (see src/pagescript.rs for the API).

// ------------------------------------------------------------------ Wasted
// The big "wasted" in the middle of the picture: first white over the scene (which turns red), then
// red on a black screen. The translation is drawn just above it, in the word's colour of the moment.

pages.option({
  id: "wasted",
  name: { en: "Wasted", ar: "انتهى أمرك (Wasted)" },
  about: { en: "Translate the big “wasted” when you die.", ar: "ترجمة كلمة «wasted» الكبيرة عند موتك." },
  on: true,
});

// the word's shape, white on black (cut from a frame of the red phase): it matches both phases
pages.image("wasted", "iVBORw0KGgoAAAANSUhEUgAAAZAAAABcCAAAAABlibxrAAARaUlEQVR42u1de5SUxZX/3ar6ZoZ5MAgIox4BUVEEjWY1Z41RCSpJ0CNG1lWCoNn4PCYmObtrdI+bZHVl3SRuzIm4B18RYiTJStCAWSVRA6LhoFEjMbiKcVCJqExgmJnu6fmq6u4f090z013fs2doOXb9M3O+rqpbVfdZ996qAmqlCkViOhsuK4Z/L2qLU41CmAcu/yzw8RpCqlROdSEEsDWEVKl0uz+LGkKqVETC77XyIUNUrdQQUis1hNQQUis1hNQQUis1hOyHhWsI+XCVUbUlwIfK2zvH5e1l5hqHVKfQ2AChpaQTJawHN1buTvXgLt39DK1DkqKHatg5UiIZ3dba8rbC3c6aJMsXx+fHxr3Gjlmzl70wUIvsM5oQMm7NcrQF0YRj9iMw8rjA4ago3DTY+qJ1iyw674RyFzzTW8tNAYHEB1yj4Or2tr3ERdvgvI85XPlMd3yQryMsMO6z06L8/VY8+kYHlC7BpcGY0V8UTBFtN7zUAcl28HrYIxY5mlmx+VdkY6JY44BZx0cNnKlzeTYjStiblI+2S0eVjkDYz50U1CG7ywkokJqHbwbU+adiHaKJAXVm5u04BXnEHd0co5iem5tQSucH3ryrN07j7kdOHWo5Kqxy18y2EMWL7WHirX/lWMD//DUMHbkETv5f96xNQCfKuOjEqsHMZ3Sf59AO6uQBvuFs1rPkMLbzpC71pLvPoliC22u88cIfLB3EI0T2nPvHAX6Mtk3nnrvmG1ulGRCTTL5WDnrulYIZ5CbTgWESY8nVY2INnJoO+/7B/7FbDCyoMBOv+nrrUD0arnKDOMTwjCKqlbiDfUcdnx8oykyFS7V21LF5DvFwdg+zb2MRGmvm6+AN0igrmH0bq7HVht89Os8j/fpkpWv0lncDkEEKRw6wPn4ae+DW+Lz9tAEGJXxmJ7PmREXtOIRdlH3JN/JfSTdfDPfABxExjpV+eSUjN7ZLCwj/nNXKyLi6UVrzn/zdAo8Ice8iHbctSfhtT85+XRoAbA6cbsd/2jV6ss3XP7NlD5o+Xj57pt7nBjhsxYV9XlzgJMyk9Yt/nB+54ItWwoikVsatLgLyeVWB+gnyfbZO6n//kALHe/g297m6eRgKkDTPtyYJmdg+/sei8rmFc4lozOe3iAgSczZsD634xoYNr7l/efqyfugKtyUFrrdP6V8ViZPYN5y04AcuhGh+tiVvPQscu9cEsOwRBdlATa+7tJTmJaQARU+40BWq2XVmMgiAoIO6jOWEGPkvIaFwCbM2Oqhxv4zVRpcXn1+DACDo0J7kwFf0m8nSe9T4nBwh03tc0tnysfnF9nB90GraIkLQ3OPiImNnQELQpE6bcFrcx1dAAfCwJKkUZmt7m0AKX9C5CKxrNwXbXM+pkICH25IDNx11IIAwro9tcoSIjgan9cedxX+zAcJO05UDtnGfW6qOAUA8ZzRTQkkq+VIYgPT4Lyd275AWC6AALcNbChlQwXrtLwgDoQ+6Ojlw2/L3kIDCYqGTzhqAyP7OtYdnOrNo0wartLHFGotbfYdpQH9uJwZhLlukcPYAAKtRKWbljQaACen35tmeAvDE0LkfONAq0zhHRNdGOFbLYk7RfOsJbNxV/K/RNW5Lf9ohLICeFGtasOHYIF1jxmQQKgsTpQY+1AhNBnm8+4c9BdOVzg0Imgic0WooNNwymtIHXajkb9LGhN9V7r+jChpRSoQ85LTTMdHjvNfnEwFdE2a2FJgr5+797ap6NBuxX4ZwX3IpXInzDtL9hG26AhvnMgVlMcWNtGVVnZvdPxHSTAFhiTwbHNocZCKxnAYCQLouYDPv1SJRiREi9r7lFLV557TEOeMDrDfSakEeD7nsR27heISksZDvPwGHLaHFl/KL3RvSOpPnohnNDvFg5EubRKiZYnzf93XKmRnf9/1qSSWtiXydUpKGDlwFxNgYB+b/qwvpuy4fbpnT6rukUzYTbl/JCsJ7LGUV+UOhV3mphVJoz4aXLnZOrT/2qOkqyMBRLbqpk3jIhmSo0g+1/Cz9ZrNgnnx+Qwqz2Ir1GwXzOTNCwvQ0UtKK8P2Ou8ZdfMgiQSkG/uKjguzsEwO8BAroYuf3i7/ZQQxwmPHYlO90jFvK3sHSBu5SmDLzuwDgifu0SjwtuuxeAPjWbz9lAjklN2IIWfQA8MGNmHmiScymlFu4FQBWz3PPWgEeOResXgIgblIhMt70lTq1hsAutPS0cTlWRH1WsMplUlAy0TrUW8heMg4tZYQFGJOMNQgmYhs4MQMTbpn+WgmfPJ1CiRj5xKujNFTWp0Adktnb4jJsTRaA1HOnBtMvy7Y3iAGnXczy7c3UP7H3lbOLHt8YZptOF7SQb2Gpw6VKZH9e4EFSImBwAEKIW6I1Avh7gtm3qWSiZV+DA31kyqptvzm/fM3J1M+/X+pQQUx+4+JvSSvN6W3GkXEiOt4hBmDo+q3lmR8WW27vIhuS5Ro1LwIs/uGeiWVDZO59GIZx64wxhJkB2yiWO94mDsLV8nAFZCv2xnBYJlGD2wU9HQSgL7T3LgCEqU3OrYoswO65886Rylju+HzwmnXMBXDkT5yC3upb7twVKluqZr8x7pnrVPhZABaXIcyMmSEtAvNBZIE2STi3nmyHI/2OA2mYCFCvrzvJFV4QmZsQ5h631TyO8CaHuKAnh6YMz68zVNgflk3p7oKqZ2MdpZCAJCpJ53d3bfuNO2utEcvdAGgM2ZASafiOHIcALS5ZKrH41iyFb9SBLAGMU5xcxO/EyHKFst2pBi6VtHFye+2ZAfLaMAWnG3O4ASW4zgKWRkZkUXv75HKUE8YKJkTsRzkDWJrnQIhV7Y8OjdEIMo5p+jgrBb0Z9dUrnWumSmJKhHNDrQZOY7qKa7+aA/CZE4wcAYRY+fa2Ka6efQ2pz5gZsmsj03LaBmF5T0ActDRFdsLcaeVZrlMWpDB8Fb6U6yldaSs2bNoDKvkczoCjvtbiysx694fEIXlj1xy3SZoJC7wRElnU7bYLx+wksAwjIdt48gaV+9gUJ/PuxqBJCauPuG5Rw/DdtyK/4vh4w85Vq56CsHHPiCl9wRLnD2+GIQQCs2aNpFLnpU7neutiSLRFtN8DwuRxDoQY3DVo+y5t/X//8fIG65cXnfLMkHb0xW3XPHnHhCQcJ3WmvJtef1cMV7PmkVLqQUK2F0xXh+6QCJPBaOOoY41kjl46m7UczrtulPvQjLhm/pmvyNi7CFbsOWISUc61kXM1i8C5CSyQmnsjWl8ifVzu9gPUDTJKznt5tk9q5IzFgfMYQrc9OQPVdM1XjBArNv1JWAf1H6nM2IkRJlCvcW/mWXWsKuyuBB/zkGf2VThX6Qnr2NJ+jBBG516nCsvkcPyMUJVLdvTh7BZq5O8oCC3B10m972hW6YNvJ7FfXxxAAYt9PDrDFReZ8WfxaW5z3NYVj61MW2D3pQyRfEWjof0YIYKXOvQ6mdbTaA7FiPU3OLP37ukrRFK4tY5oHybzFHJ79+OrNQI2Tx7PjnCCSvoyzQpKgKACiKs4ve9UNKRo5LXs53ed9PpO1+HC0VNLs7/LPKSC57kFXvcA1zVS+lSGzmfSOIX1fo0QTY+9phxmFk++dupQWcOZ/6ESU2zCDTNZOoysvSsGuKkCX7bIPEZ2H+XkfphuA2pwxq3H3mxLUgt2Ltw6hGAJ45fUOWdfPzzj03Rf1rPAR+t6JoEfO3frXGLzMrb7S0osZLdAYbQPj53D9JfPZoT9iCGE8Ko7nVSUavSfUntEnULNlRk1LL4eKzd8rkcY/ohdYDYqnobNtHNcUTQ2ZoYsR4c+Npy9R5J1HfZDNXN7g48ZDgNCTDyEdKwTz/4+judOmM0xVLnxiSiS+LVcf8zndwtF5aVaub2sNQUVRY0V+0w1Vt1yaJywhJGc7Y2Fut61MRAiJXrQFCNC9+7Dz3zqzIVlez1VP5LR7fD7gTJBdGRyz1Vycis/yUwf4oQul5n6mCcPVHNXdDrorzb/dgvN/15LlJVqBX2wevWNZWb52IUXHlOF5wSY+Dsd9+uAEBbpvZXkEBXua2iOVdsH4YdnxMH/Xj/S6UKZi7oB3H3EdZGhaQsStLvs8+6b7tp42L7HCNPClaHy2lasQwSvjiFhyOx2Xy/mMLIe3BVpZBHqPUmeejYOh7NxSe2Gne2Cq2AIPaXqKLjYyuNuwr4cvSqsOh5EHz23fVIMwV0Xi9R8YtYRSsSdLd1vyvhUh2qU5p2CA03+ypIeVSElMxfDQc4CjL/smhwZ/iG8GV+x2Qp+rpLlays5xh4HIRYPLRsTTfceANCoOLegrhimZExhj5rqytPZ8TJV/RJJHvcJ15r1PjUMqQJCvXJKFEKMeqBXaWnuuj2GHm0Ytklvcp4G8o9qFzaeQhsxVWJWzHX+cMFDYVu1iCHnRZbq+8UpJuqaR7xrJQhvRW+u5cNvJ0j8CP3Ra9Yu577XwASA+6pxVUN/DgVTm3aQpl8/MbTXHAd7nAYnnDTHHn0rRS/xLj9WxM4jVjYiWdXUO1hXMClpCROmpN4ZkkesbLrAyV4lrbAnzXDZFCz80CU8anyXsJ7ORt393sORt9x03wcNi+f/Gm1qjollzhu/z8/aa8OvQHIuuKGv65zf1zf98JTbECbf7/OzdjSnUeln65yf81vqk3OdsMdMyfXp7PgTWIRxiMbybzdHEduo/hMjf3xvbISZJXhttJFFtvGR5wXzpJNSrKnCF003gc9OeVc62Za1Lwrmw85mL4WYu/fEDIjnpTpJwff+msjMCXIxFCVLzkT6wddlpQEgRkcfyXw6htUrcPrp6Q+IyCsr3NzNnp1e7/DVqTWQwHHHhWU0Fz/LUZGM+kKvACDtzyJMWkZnvAPcxvd9P60tryu8yaEfeDrbmSoB3n+TQ4RSZ9X9i4tMhCI+IE8VWyMQYtS6bSqOtqwoQ7bSPJ+qAQ9PcBZFnP9fhJRRtn+zx3gvF6XVm1ErFT95NDbStfgGGIDFmg/CHYeEP9QWtlKEWDwfZex4hUiYJ6O21w+Ca0tbKUJ+aUNv0bTY0pmXVBx1LT61JtiLo0qvb33oRdaoXISRtbFTMgD29D2hHmamN9uHev7UPr+oj4cJOFfQiCtCCHs7Hwx35PNADLArIjS79Z2hQbPOCqg0VW7vYCykAm4rea9HVXK/oYgbWlC0rOj+jzKi6qnkBoHUDxGx3PN08rQaKl5yC8ayFGdF8ndSkd+TJuCyJx9iSnOx9QBCGO+FbTwZnYXrkA3Wdodf2jz0+D7LbVtt2u2fyD6eOCRq5dafkC6OO2eTO7oegQCs2rmUkg7ceq/+nAxg8ICSXJEymRx+6f+DAw9YqK6wa+a1XT/0jJ/CoqSvI7DhV/pvPBVt3Zzw0Yde/rcBeSGxPPHTDKZzbD9wOiSTFHiOL0b+dQT3QzhRryMMIGRqOEJWFhFCjX8IG6bmy4cihGTDmsSLkkcIBD7ZlWxRfN5+9MBumETTY8mAa81X5CWHxOk9yYD3Ft8dIonHEz4+UoKQw8Nn+UhRU3r4l5ApWtMxrURsE6k1CR83KSIEEp/cm+DRCG14++GD/ZWEuseSADfMVxWnKjGrh7WN/VSF4W1TCsQgxcx32WibEiGQdWutDiH7fx2EkO/ZvpCaz5b5bwWpNcycYHCGtxRUmsIpHSHPmpW9ffadcUMZVMB7PC5wbZhfPn5Qe4XTdycB/u8HDGhiwoSlnPQRqsGS/kchMs/w0cVhSvwdh6FuY7ldQ6TOX9+TiEPeKM5M4sAHujne6yh61Sll/nxB3gUbMzEBd/9z4xB8Ckz4eVzg3a/97RDgApj1y65kD9kMPldw5Lbgl6IM/83AOIXYFIwRn7/g2IwRQNO++2psgaz5/kGhAeDg8zYbE62NH5/lSuQiQEy/7fVI4Mau/srBJU8RQoIOnv9CJHBr99zQ1lwCnARw0C1ZG1/9Lft/PIpQd8HJ1WMAAAAASUVORK5CYII=");
const TPL_W = 400;

// "Wasted", as the title over it: short, and for anyone (no he / she)
const WASTED = { ar: "انتهى أمرك" };

const WHITE = "#ffffff";
const RED = "#d7303a";

/** The big word's box (frame pixels) and colour, or null. */
function findWasted(f) {
  const R = { x: Math.floor(f.w * 0.2), y: Math.floor(f.h * 0.3), w: Math.floor(f.w * 0.6), h: Math.floor(f.h * 0.4) };
  const step = 3;
  const looksLikeTheWord = (m) => {
    if (!m || !m.box) return false;
    const b = m.box;
    const cx = (b.x + b.w / 2) / f.w, cy = (b.y + b.h / 2) / f.h;
    const bw = b.w / f.w, bh = b.h / f.h;
    // centred, a little above the middle, one big word (measured: x 0.33–0.67, y 0.43–0.55)
    if (Math.abs(cx - 0.5) > 0.03 || cy < 0.42 || cy > 0.56 || bw < 0.25 || bw > 0.45 || bh < 0.08 || bh > 0.18) return false;
    // heavy letters fill much of their box
    const fill = (m.count * step * step) / (b.w * b.h);
    return fill > 0.25 && fill < 0.9;
  };
  // white letters, over whatever the scene is
  const white = f.pixels(R, { step, bright: [225, 255], spread: [0, 40] });
  if (looksLikeTheWord(white)) return { box: white.box, color: WHITE };
  // red letters on a black screen
  const red = f.pixels(R, { step, near: [215, 48, 56], tol: 60 });
  if (looksLikeTheWord(red)) {
    const top = f.pixels({ x: 0, y: 0, w: f.w, h: Math.floor(f.h * 0.25) }, { step: 8, bright: [60, 255] });
    const all = Math.floor(f.w / 8) * Math.floor((f.h * 0.25) / 8);
    if (top.count < all * 0.05) return { box: red.box, color: RED };
  }
  return null;
}

pages.add({
  id: "gta6_wasted",
  option: "wasted",

  present(f) {
    return findWasted(f);
  },

  // Only "wasted" itself: a mission's big title in the same place is not this page.
  read(f, k) {
    if (!k) return null;
    const b = k.box;
    const pad = Math.floor(b.h * 0.15);
    // (too big and too stylised for the OCR: its shape is compared instead)
    const s = b.w / TPL_W;
    const hit = f.find({ x: Math.max(b.x - pad, 0), y: Math.max(b.y - pad, 0), w: b.w + 2 * pad, h: b.h + 2 * pad }, "wasted", { scales: [s * 0.94, s, s * 1.06], step: 2 });
    if (!hit || hit.score < 0.75) return null;
    // the word in the player's language (others: the app's translator)
    const ar = WASTED[vi.lang()] || "";
    return { title: f.box(b.x, b.y, b.w, b.h), title_color: k.color, title_en: "Wasted", title_ar: ar };
  },

  // While it's up: the same word in the same colour (white → red reads it again, in red).
  same(f, old) {
    const k = findWasted(f);
    if (!k || k.color !== old.title_color) return false;
    const r = f.box(k.box.x, k.box.y, k.box.w, k.box.h), t = old.title, tol = 0.02;
    return Math.abs(r.x - t.x) < tol && Math.abs(r.y - t.y) < tol && Math.abs(r.w - t.w) < tol;
  },
});
