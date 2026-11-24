// The PAC builtins that read the clock. They answer as Chromium's PAC library does (Firefox
// shares it), so a script behaves as it does in a browser:
//   timeRange(9, 17) runs to 17:59:59 and the hour form does not wrap;
//   dateRange(24, "DEC") keeps both ends in the current month;
//   an unknown day or month name is false.
var __weekdays = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
var __months = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
var __clock = function () { return new Date(); };

function __split(args) {
  var values = Array.prototype.slice.call(args);
  var last = values[values.length - 1];
  var gmt = typeof last === "string" && last.toUpperCase() === "GMT";
  if (gmt) { values.pop(); }
  return { values: values, gmt: gmt };
}

function __now(gmt) {
  var d = __clock();
  if (gmt) {
    return { year: d.getUTCFullYear(), month: d.getUTCMonth() + 1, day: d.getUTCDate(),
             weekday: d.getUTCDay(), hour: d.getUTCHours(), minute: d.getUTCMinutes(),
             second: d.getUTCSeconds() };
  }
  return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate(),
           weekday: d.getDay(), hour: d.getHours(), minute: d.getMinutes(),
           second: d.getSeconds() };
}

function __compare(a, b) {
  for (var i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) { return a[i] < b[i] ? -1 : 1; }
  }
  return 0;
}

function __within(start, end, moment) {
  if (__compare(start, end) <= 0) {
    return __compare(start, moment) <= 0 && __compare(moment, end) <= 0;
  }
  return __compare(moment, start) >= 0 || __compare(moment, end) <= 0;
}

function weekdayRange() {
  var split = __split(arguments);
  var names = split.values.slice(0, 2).map(function (v) { return String(v).toUpperCase(); });
  if (names.length === 0) { return false; }
  for (var i = 0; i < names.length; i++) {
    if (__weekdays.indexOf(names[i]) < 0) { return false; }
  }
  var start = __weekdays.indexOf(names[0]);
  var end = __weekdays.indexOf(names[names.length - 1]);
  var today = __now(split.gmt).weekday;
  return start <= end ? (start <= today && today <= end) : (today >= start || today <= end);
}

function __datePart(value) {
  if (typeof value === "string" && !/^\s*\d+\s*$/.test(value)) {
    var month = __months.indexOf(value.trim().toUpperCase());
    if (month < 0) { throw new Error("month"); }
    return ["month", month + 1];
  }
  var number = parseInt(value, 10);
  if (isNaN(number)) { throw new Error("number"); }
  return number < 32 ? ["day", number] : ["year", number];
}

function dateRange() {
  var split = __split(arguments);
  var values = split.values;
  if (values.length === 0) { return false; }
  var now = __now(split.gmt);
  var parts = [];
  try {
    for (var i = 0; i < values.length; i++) { parts.push(__datePart(values[i])); }
  } catch (e) {
    return false;
  }
  if (parts.length === 1) {
    var current = { day: now.day, month: now.month, year: now.year };
    return current[parts[0][0]] === parts[0][1];
  }
  var start = [now.year, 1, 1, 0, 0, 0];
  var end = [now.year, 12, 31, 23, 59, 59];
  var index = { year: 0, month: 1, day: 2 };
  var half = Math.floor(parts.length / 2);
  var currentMonth = false;
  for (var s = 0; s < half; s++) {
    start[index[parts[s][0]]] = parts[s][1];
    currentMonth = currentMonth || (parts[s][0] === "day" && parts.length <= 2);
  }
  for (var e = half; e < parts.length; e++) {
    end[index[parts[e][0]]] = parts[e][1];
  }
  if (currentMonth) { start[1] = now.month; end[1] = now.month; }
  return __within(start, end, [now.year, now.month, now.day, now.hour, now.minute, now.second]);
}

function timeRange() {
  var split = __split(arguments);
  var numbers = split.values.map(function (v) { return parseInt(v, 10); });
  var now = __now(split.gmt);
  if (numbers.length === 1) { return now.hour === numbers[0]; }
  if (numbers.length === 2) { return numbers[0] <= now.hour && now.hour <= numbers[1]; }
  var start;
  var end;
  if (numbers.length === 4) {
    start = [numbers[0], numbers[1], 0];
    end = [numbers[2], numbers[3], 59];
  } else if (numbers.length === 6) {
    start = numbers.slice(0, 3);
    end = numbers.slice(3);
  } else {
    return false;
  }
  return __within(start, end, [now.hour, now.minute, now.second]);
}
