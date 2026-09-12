#pragma once
#include <google/protobuf/descriptor.h>
#include <google/protobuf/message.h>
#include <string>
#include <string_view>

namespace rx::wire {
// Structural validation only. Application identity/authorization/profile checks remain mandatory.
bool validate(const google::protobuf::Descriptor& descriptor, std::string_view bytes, std::string& error);
bool parse(google::protobuf::Message& message, std::string_view bytes, std::string& error);
}
