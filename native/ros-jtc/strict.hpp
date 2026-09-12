#pragma once
#include <algorithm>
#include <array>
#include <charconv>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <nlohmann/json.hpp>
#include <random>
#include <regex>
#include <set>
#include <time.h>
namespace rx::ros_bridge {
using Json = nlohmann::json;
inline void require(bool value, const char *reason) {
  if (!value)
    throw std::invalid_argument(reason);
}
inline void keys(const Json &v, std::initializer_list<const char *> names) {
  require(v.is_object() && v.size() == names.size(), "field set differs");
  for (auto name : names)
    require(v.contains(name), "field missing");
}
inline std::string text(const Json &v) {
  require(v.is_string(), "string required");
  return v.get<std::string>();
}
inline std::string uuid(const Json &v) {
  auto s = text(v);
  static const std::regex p(
      "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}");
  require(std::regex_match(s, p) && s != "00000000-0000-0000-0000-000000000000",
          "nonzero UUID required");
  return s;
}
inline std::string new_uuid() {
  std::random_device random;
  std::array<unsigned char, 16> v{};
  for (auto &b : v)
    b = static_cast<unsigned char>(random());
  v[6] = (v[6] & 15) | 64;
  v[8] = (v[8] & 63) | 128;
  const char *h = "0123456789abcdef";
  std::string s;
  for (std::size_t i = 0; i < v.size(); ++i) {
    if (i == 4 || i == 6 || i == 8 || i == 10)
      s += '-';
    s += h[v[i] >> 4];
    s += h[v[i] & 15];
  }
  return s;
}
inline uint64_t counter(const Json &v) {
  auto s = text(v);
  require(!s.empty() && (s.size() == 1 || s[0] != '0'),
          "canonical decimal counter required");
  uint64_t value = 0;
  auto p = std::from_chars(s.data(), s.data() + s.size(), value);
  require(p.ec == std::errc{} && p.ptr == s.data() + s.size(),
          "uint64 required");
  return value;
}
inline unsigned integer(const Json &v, unsigned maximum) {
  require(v.is_number_unsigned() || v.is_number_integer(), "integer required");
  require(v >= 0 && v <= maximum, "integer range");
  return v.get<unsigned>();
}
inline double number(const Json &v) {
  require(v.is_number(), "number required");
  double n = v.get<double>();
  require(std::isfinite(n) && std::abs(n) <= 1e6,
          "bounded finite number required");
  return n;
}
inline Json parse(const std::string &text) {
  std::vector<std::set<std::string>> objects;
  return Json::parse(text, [&](int depth, Json::parse_event_t event, Json &v) {
    require(depth <= 32, "JSON nesting limit");
    if (event == Json::parse_event_t::object_start)
      objects.emplace_back();
    if (event == Json::parse_event_t::key)
      require(!objects.empty() &&
                  objects.back().insert(v.get<std::string>()).second,
              "duplicate JSON key");
    if (event == Json::parse_event_t::object_end)
      objects.pop_back();
    return true;
  });
}
inline Json file(const std::string &name) {
  require(
      std::filesystem::is_regular_file(std::filesystem::symlink_status(name)),
      "regular configuration file required");
  std::ifstream input(name);
  require(input.good(), "configuration unreadable");
  std::string value;
  char c;
  while (input.get(c)) {
    require(value.size() < 1048576, "configuration too large");
    value += c;
  }
  return parse(value);
}
inline bool packet(std::string &s) {
  s.clear();
  char c;
  while (std::cin.get(c)) {
    if (c == '\n')
      return true;
    require(s.size() < 1048576, "IPC packet limit");
    s += c;
  }
  require(s.empty(), "truncated IPC packet");
  return false;
}
inline uint64_t now() {
  timespec ts{};
  require(clock_gettime(CLOCK_BOOTTIME, &ts) == 0 && ts.tv_sec >= 0,
          "boottime unavailable");
  return static_cast<uint64_t>(ts.tv_sec) * 1000000000ULL +
         static_cast<uint64_t>(ts.tv_nsec);
}
inline std::string clock_id() {
  std::ifstream f("/proc/sys/kernel/random/boot_id");
  std::string id;
  f >> id;
  return "linux-boottime/" + uuid(Json(id));
}
} // namespace rx::ros_bridge
