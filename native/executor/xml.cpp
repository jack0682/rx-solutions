#include "runtime.hpp"
#include <tinyxml2.h>
#include <charconv>
#include <stdexcept>

namespace rx::bt {
namespace {
std::string attribute(const tinyxml2::XMLElement* element, const char* name) {
  const auto* value = element->Attribute(name);
  if (!value) throw std::invalid_argument(std::string("missing XML attribute: ") + name);
  return value;
}
void attributes(const tinyxml2::XMLElement* element, const std::set<std::string>& names) {
  for (auto* item = element->FirstAttribute(); item; item = item->Next())
    if (!names.count(item->Name())) throw std::invalid_argument("unapproved XML attribute/script");
}
void children_only(const tinyxml2::XMLNode* parent) {
  for (auto* child = parent->FirstChild(); child; child = child->NextSibling())
    if (!child->ToElement() && !child->ToComment()) throw std::invalid_argument("unapproved XML text/declaration");
}
std::string tag(Kind kind) {
  switch (kind) {
    case Kind::Sequence: return "RXSequence";
    case Kind::ParallelAll: return "RXParallelAll";
    case Kind::Branch: return "RXBranch";
    case Kind::Operation: return "RXOperation";
    case Kind::Wait: return "RXWait";
    case Kind::Intervention: return "RXIntervention";
  }
  throw std::invalid_argument("unknown RX kind");
}
void node(const tinyxml2::XMLElement* element, const Context& context, const std::string& expected,
          std::set<std::string>& visited, std::size_t depth) {
  if (!element || depth > 64 || visited.size() >= 4096) throw std::invalid_argument("XML shape/size");
  const auto id = attribute(element, "node_id");
  if (id != expected || !visited.insert(id).second) throw std::invalid_argument("XML node identity/order mismatch");
  const auto& definition = context.definition(id);
  if (element->Name() != tag(definition.kind)) throw std::invalid_argument("XML node kind mismatch");
  std::set<std::string> allowed{"node_id"};
  const char* argument = nullptr;
  switch (definition.kind) {
    case Kind::Operation: argument = "binding"; break;
    case Kind::Branch: case Kind::Wait: argument = "condition_id"; break;
    case Kind::Intervention: argument = "procedure_digest"; break;
    default: break;
  }
  if (argument) {
    allowed.insert(argument);
    if (attribute(element, argument) != definition.argument) throw std::invalid_argument("XML binding differs from resolved process");
  }
  if (definition.kind == Kind::Wait) {
    allowed.insert("timeout_ns"); const auto value = attribute(element, "timeout_ns"); std::uint64_t parsed = 0;
    const auto result = std::from_chars(value.data(), value.data() + value.size(), parsed);
    if (result.ec != std::errc{} || result.ptr != value.data() + value.size() || parsed == 0 || parsed != definition.timeout_ns)
      throw std::invalid_argument("XML wait limit differs from resolved process");
  }
  attributes(element, allowed); children_only(element);
  auto* child = element->FirstChildElement();
  for (const auto& expected_child : definition.children) {
    node(child, context, expected_child, visited, depth + 1); child = child->NextSiblingElement();
  }
  if (child) throw std::invalid_argument("extra XML child");
  if (definition.kind == Kind::Branch && definition.children.size() != 2) throw std::invalid_argument("branch child count");
}
}  // namespace
void validate_xml(const std::string& xml, const Context& context) {
  if (xml.size() > 1'048'576) throw std::invalid_argument("XML size limit");
  tinyxml2::XMLDocument document;
  if (document.Parse(xml.data(), xml.size()) != tinyxml2::XML_SUCCESS) throw std::invalid_argument("invalid XML");
  children_only(&document);
  auto* root = document.RootElement();
  if (!root || std::string(root->Name()) != "root" || root->NextSiblingElement()) throw std::invalid_argument("XML root");
  attributes(root, {"BTCPP_format", "main_tree_to_execute"}); children_only(root);
  if (attribute(root, "BTCPP_format") != "4" || attribute(root, "main_tree_to_execute") != "RXMain") throw std::invalid_argument("BT XML version/tree");
  auto* tree = root->FirstChildElement();
  if (!tree || std::string(tree->Name()) != "BehaviorTree" || tree->NextSiblingElement()) throw std::invalid_argument("one resolved BehaviorTree required");
  attributes(tree, {"ID"}); children_only(tree);
  if (attribute(tree, "ID") != "RXMain") throw std::invalid_argument("tree ID");
  auto* first = tree->FirstChildElement();
  if (!first || first->NextSiblingElement()) throw std::invalid_argument("one root node required");
  std::set<std::string> visited; node(first, context, context.root(), visited, 0);
  if (visited.size() != context.definitions().size()) throw std::invalid_argument("resolved nodes missing from XML");
}
}  // namespace rx::bt
