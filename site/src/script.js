(function () {
  "use strict";

  var root = document.documentElement;
  var STORAGE_KEY = "breakbar-theme";

  function applyTheme(theme) {
    root.setAttribute("data-theme", theme);
    root.style.colorScheme = theme === "light" ? "light" : "dark";
    document.querySelectorAll(".theme-toggle").forEach(function (btn) {
      // The texts come from the page, so that they are in its language.
      btn.setAttribute(
        "aria-label",
        theme === "dark" ? btn.getAttribute("data-to-light") : btn.getAttribute("data-to-dark")
      );
    });
  }

  function getStoredTheme() {
    try {
      var stored = localStorage.getItem(STORAGE_KEY);
      if (stored === "dark" || stored === "light") return stored;
    } catch (e) {
      /* localStorage unavailable — fall through */
    }
    return null;
  }

  function getPreferredTheme() {
    var stored = getStoredTheme();
    if (stored) return stored;
    if (window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches) {
      return "light";
    }
    return "dark";
  }

  function toggleTheme() {
    var current = root.getAttribute("data-theme") === "light" ? "light" : "dark";
    var next = current === "dark" ? "light" : "dark";
    applyTheme(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch (e) {
      /* ignore */
    }
  }

  applyTheme(getPreferredTheme());

  // Follow the OS preference live, but only while the person hasn't chosen a theme themselves.
  if (window.matchMedia) {
    var media = window.matchMedia("(prefers-color-scheme: light)");
    var onMediaChange = function (e) {
      if (getStoredTheme()) return;
      applyTheme(e.matches ? "light" : "dark");
    };
    if (media.addEventListener) media.addEventListener("change", onMediaChange);
    else if (media.addListener) media.addListener(onMediaChange);
  }

  document.querySelectorAll("#theme-toggle, #theme-toggle-mobile").forEach(function (btn) {
    btn.addEventListener("click", toggleTheme);
  });

  // Mobile nav
  var menuToggle = document.getElementById("menu-toggle");
  var mobileNav = document.getElementById("mobile-nav");
  if (menuToggle && mobileNav) {
    menuToggle.addEventListener("click", function () {
      var open = mobileNav.classList.toggle("open");
      menuToggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
    mobileNav.querySelectorAll("a").forEach(function (link) {
      link.addEventListener("click", function () {
        mobileNav.classList.remove("open");
        menuToggle.setAttribute("aria-expanded", "false");
      });
    });
  }

  // Hero screenshot dark/light toggle
  var shotButtons = document.querySelectorAll(".shot-btn");
  shotButtons.forEach(function (btn) {
    btn.addEventListener("click", function () {
      var which = btn.getAttribute("data-shot");
      shotButtons.forEach(function (b) {
        b.classList.toggle("active", b === btn);
      });
      document.querySelectorAll(".shot-img").forEach(function (img) {
        img.classList.toggle("active", img.classList.contains("shot-" + which));
      });
    });
  });
})();
