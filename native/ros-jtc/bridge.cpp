#include "catalog.hpp"
#include "strict.hpp"
#include <action_msgs/srv/cancel_goal.hpp>
#include <control_msgs/action/follow_joint_trajectory.hpp>
#include <controller_manager_msgs/srv/list_controllers.hpp>
#include <map>
#include <rclcpp/rclcpp.hpp>
using namespace rx::ros_bridge;
using Action = control_msgs::action::FollowJointTrajectory;
using Send = Action::Impl::SendGoalService;
using Result = Action::Impl::GetResultService;
using Cancel = action_msgs::srv::CancelGoal;
using List = controller_manager_msgs::srv::ListControllers;
namespace {
std::array<uint8_t, 16> goal_id(const std::string &s) {
  std::array<uint8_t, 16> out{};
  std::string hex;
  for (char c : s)
    if (c != '-')
      hex += c;
  require(hex.size() == 32, "UUID bytes");
  for (std::size_t i = 0; i < 16; ++i)
    out[i] =
        static_cast<uint8_t>(std::stoul(hex.substr(i * 2, 2), nullptr, 16));
  return out;
}
std::string ros_name(const Json &value, bool allow_root = false) {
  auto s = text(value);
  static const std::regex pattern(
      "/([A-Za-z_][A-Za-z0-9_]*)(/[A-Za-z_][A-Za-z0-9_]*)*");
  require(s.size()<=256&&((allow_root && s == "/") || std::regex_match(s, pattern)),
          "absolute ROS name required");
  return s;
}
builtin_interfaces::msg::Duration duration(uint64_t n) {
  require(n <= 3600000000000ULL, "duration exceeds one hour");
  builtin_interfaces::msg::Duration d;
  d.sec = static_cast<int32_t>(n / 1000000000ULL);
  d.nanosec = static_cast<uint32_t>(n % 1000000000ULL);
  return d;
}
std::vector<double> vector(const Json &v, std::size_t count,
                           bool optional = false) {
  require(v.is_array() && (v.size() == count || (optional && v.empty())),
          "joint vector shape");
  std::vector<double> result;
  for (const auto &x : v)
    result.push_back(number(x));
  return result;
}
std::vector<control_msgs::msg::JointTolerance>
tolerances(const Json &v, const std::vector<std::string> &joints) {
  require(v.is_array() && v.size() == joints.size(),
          "full explicit tolerances required");
  std::vector<control_msgs::msg::JointTolerance> out;
  for (std::size_t i = 0; i < joints.size(); ++i) {
    const auto &t = v[i];
    keys(t, {"name", "position", "velocity", "acceleration"});
    require(text(t["name"]) == joints[i], "tolerance joint order differs");
    control_msgs::msg::JointTolerance x;
    x.name = joints[i];
    x.position = number(t["position"]);
    x.velocity = number(t["velocity"]);
    x.acceleration = number(t["acceleration"]);
    require(x.position > 0 && x.velocity > 0 && x.acceleration > 0,
            "default or disabled tolerance not allowed");
    out.push_back(x);
  }
  return out;
}
Action::Goal goal(const Json &v, const std::vector<std::string> &joints) {
  keys(v, {"joints", "points", "path_tolerance", "goal_tolerance",
           "goal_time_ns"});
  require(v["joints"] == Json(joints), "catalog joint order differs");
  Action::Goal out;
  out.trajectory.joint_names = joints;
  require(v["points"].is_array() && !v["points"].empty() &&
              v["points"].size() <= 1024,
          "trajectory point count");
  uint64_t previous = 0;
  for (const auto &p : v["points"]) {
    keys(p, {"positions", "velocities", "accelerations", "time_ns"});
    trajectory_msgs::msg::JointTrajectoryPoint point;
    point.positions = vector(p["positions"], joints.size());
    point.velocities = vector(p["velocities"], joints.size(), true);
    point.accelerations = vector(p["accelerations"], joints.size(), true);
    auto t = counter(p["time_ns"]);
    require(t > previous, "trajectory times must strictly increase from zero");
    point.time_from_start = duration(t);
    previous = t;
    out.trajectory.points.push_back(point);
  }
  out.path_tolerance = tolerances(v["path_tolerance"], joints);
  out.goal_tolerance = tolerances(v["goal_tolerance"], joints);
  auto tolerance = counter(v["goal_time_ns"]);
  require(tolerance > 0 && tolerance <= 60000000000ULL,
          "bounded goal time tolerance required");
  out.goal_time_tolerance = duration(tolerance);
  return out;
}
struct Entry {
  std::string operation, body;
  Json send;
  Json result = nullptr;
  bool pending = true;
  Json cancel = nullptr;
};
class Bridge {
  rclcpp::Node::SharedPtr node_;
  rclcpp::Client<Send>::SharedPtr send_;
  rclcpp::Client<Result>::SharedPtr result_;
  rclcpp::Client<Cancel>::SharedPtr cancel_;
  rclcpp::Client<List>::SharedPtr list_;
  std::string instance_ = new_uuid(), clock_ = clock_id(), controller_, action_,
              support_, model_;
  std::vector<std::string> joints_, interfaces_;
  unsigned timeout_ = 0, capacity_ = 0;
  uint64_t sequence_ = 0;
  std::size_t recorded_bytes_ = 0;
  bool faulted_ = false;
  std::map<std::string, Entry> goals_;
  std::map<std::string, std::string> operations_;
  void deadline(uint64_t end) {
    require(now() < end, "request deadline expired");
  }
  std::chrono::nanoseconds remaining(uint64_t end) {
    auto at = now();
    require(at < end, "request deadline expired");
    return std::chrono::nanoseconds(std::min<uint64_t>(
        end - at, static_cast<uint64_t>(timeout_) * 1000000ULL));
  }
  template <class Service>
  typename Service::Response::SharedPtr
  call(typename rclcpp::Client<Service>::SharedPtr client,
       typename Service::Request::SharedPtr request, uint64_t end) {
    if (!client->wait_for_service(remaining(end))) {
      throw std::runtime_error("ROS service unavailable before deadline");
    }
    auto wait = remaining(end);
    auto future = client->async_send_request(request);
    auto status = rclcpp::spin_until_future_complete(node_, future, wait);
    if (status != rclcpp::FutureReturnCode::SUCCESS) {
      client->remove_pending_request(future.request_id);
      throw std::runtime_error("ROS response unknown or still pending");
    }
    return future.get();
  }
  Json inspect(uint64_t end, bool require_active) {
    const auto start = now();
    auto response = call<List>(list_, std::make_shared<List::Request>(), end);
    require(response->controller.size() <= 128,
            "controller observation count limit");
    Json rows = Json::array();
    bool matched = false;
    std::set<std::string> names;
    for (const auto &c : response->controller) {
      require(c.name.size() <= 128 && c.type.size() <= 256 &&
                  c.state.size() <= 32 && c.claimed_interfaces.size() <= 192,
              "controller observation size limit");
      for (const auto &interface : c.claimed_interfaces)
        require(interface.size() <= 256, "claimed interface size limit");
      require(names.insert(c.name).second, "duplicate controller observation");
      rows.push_back({{"name", c.name},
                      {"type", c.type},
                      {"state", c.state},
                      {"claimed_interfaces", c.claimed_interfaces},
                      {"is_chained", c.is_chained}});
      if (c.name == controller_) {
        std::set<std::string> expected;
        for (const auto &joint : joints_)
          for (const auto &interface : interfaces_)
            expected.insert(joint + "/" + interface);
        matched =
            c.type == "joint_trajectory_controller/JointTrajectoryController" &&
            c.state == "active" && !c.is_chained &&
            std::set<std::string>(c.claimed_interfaces.begin(),
                                  c.claimed_interfaces.end()) == expected &&
            c.claimed_interfaces.size() == expected.size();
      }
    }
    if (require_active)
      require(matched, "active controller/type/claimed interfaces differ");
    return {{"controllers", rows},
            {"selected_matches", matched},
            {"observed_start_ns", std::to_string(start)},
            {"observed_end_ns", std::to_string(now())},
            {"controller_generation_known", false},
            {"send_service_ready", send_->service_is_ready()},
            {"result_service_ready", result_->service_is_ready()},
            {"cancel_service_ready", cancel_->service_is_ready()},
            {"physical_readiness_proven", false}};
  }

public:
  explicit Bridge(const Json &config) {
    keys(config, {"schema", "catalog_sha256", "support_id", "controller", "namespace",
                  "controller_manager", "domain_id", "timeout_ms", "capacity"});
    require(text(config["schema"]) == "rx.ros-jtc-bridge.v1",
            "bridge configuration schema");
    require(text(config["catalog_sha256"])==catalog_sha256,"catalog source digest differs");
    support_ = text(config["support_id"]);
    controller_ = text(config["controller"]);
    timeout_ = integer(config["timeout_ms"], 1000);
    capacity_ = integer(config["capacity"], 512);
    require(timeout_ > 0 && capacity_ > 0, "positive bounds required");
    auto catalog = parse(catalog_json);
    bool found = false;
    for (const auto &p : catalog["profiles"])
      if (text(p["support_id"]) == support_) {
        require(p["role"] == "MANIPULATOR" || p["role"] == "FOLLOWER" ||
                    p["role"] == "MOBILE_BASE",
                "support role does not provide production JTC");
        model_ = text(p["model"]);
        for (const auto &c : p["controllers"])
          if (text(c["name"]) == controller_) {
            require(c["plugin"] ==
                        "joint_trajectory_controller/JointTrajectoryController",
                    "unsupported controller kind");
            joints_ = c["joint_order"].get<std::vector<std::string>>();
            interfaces_ =
                c["command_interfaces"].get<std::vector<std::string>>();
            found = true;
          }
      }
    require(found && !joints_.empty() && joints_.size() <= 64,
            "catalog controller/joints absent");
    require(std::set<std::string>(joints_.begin(), joints_.end()).size() ==
                joints_.size(),
            "duplicate catalog joints");
    require(interfaces_ == std::vector<std::string>{"position"},
            "only position-command JTC binding supported");
    auto ns = ros_name(config["namespace"], true);
    action_ =
        (ns == "/" ? "" : ns) + "/" + controller_ + "/follow_joint_trajectory";
    auto manager = ros_name(config["controller_manager"]);
    rclcpp::InitOptions options;
    options.set_domain_id(integer(config["domain_id"], 232));
    rclcpp::init(0, nullptr, options);
    node_ = std::make_shared<rclcpp::Node>(
        "rx_jtc_bridge", rclcpp::NodeOptions()
                             .use_global_arguments(false)
                             .enable_rosout(false)
                             .start_parameter_services(false)
                             .start_parameter_event_publisher(false));
    send_ = node_->create_client<Send>(action_ + "/_action/send_goal");
    result_ = node_->create_client<Result>(action_ + "/_action/get_result");
    cancel_ = node_->create_client<Cancel>(action_ + "/_action/cancel_goal");
    list_ = node_->create_client<List>(manager + "/list_controllers");
  }
  void reply(const Json &seq, const char *state, Json value,
             Json fault = nullptr) {
    auto message = Json({{"schema", "rx.ros-jtc-reply.v1"},
                         {"bridge_instance", instance_},
                         {"sequence", seq},
                         {"clock_id", clock_},
                         {"ticks_ns", std::to_string(now())},
                         {"state", state},
                         {"value", value},
                         {"fault", fault}})
                       .dump(-1, ' ', false, Json::error_handler_t::replace);
    require(message.size() <= 1048576, "IPC response limit");
    std::cout << message << '\n' << std::flush;
    require(std::cout.good(), "parent output closed");
  }
  void hello() {
    reply(nullptr, "READY",
          {{"support_id", support_},
           {"model", model_},
           {"controller", controller_},
           {"action", action_},
           {"joints", joints_},
           {"source_observed_only", true},
           {"catalog_sha256", catalog_sha256},
           {"controller_generation_known", false}});
  }
  void handle(const Json &p) {
    keys(p, {"schema", "bridge_instance", "sequence", "expires_at_ns",
             "command", "body"});
    require(text(p["schema"]) == "rx.ros-jtc-request.v1" &&
                text(p["bridge_instance"]) == instance_,
            "bridge request context differs");
    auto seq = counter(p["sequence"]);
    require(seq > sequence_, "IPC sequence did not advance");
    sequence_ = seq;
    auto end = counter(p["expires_at_ns"]);
    auto at = now();
    require(end > at && end - at <= 1000000000ULL,
            "one-second maximum admission window");
    const auto &body = p["body"];
    auto cmd = text(p["command"]);
    if (cmd == "inspect") {
      keys(body, {});
      reply(p["sequence"], "OBSERVED", inspect(end, false));
      return;
    }
    if (cmd == "send") {
      keys(body, {"operation", "invocation", "goal"});
      auto operation = uuid(body["operation"]),
           invocation = uuid(body["invocation"]);
      auto payload = body["goal"].dump();
      auto native = goal(body["goal"], joints_);
      auto existing = goals_.find(invocation);
      if (existing != goals_.end()) {
        require(existing->second.operation == operation &&
                    existing->second.body == payload,
                "invocation/body conflict");
        reply(p["sequence"], "SEND_RECORDED", existing->second.send);
        return;
      }
      require(!faulted_ && goals_.size() < capacity_ &&
                  recorded_bytes_ + payload.size() <= 16 * 1024 * 1024 &&
                  !operations_.count(operation),
              "bridge admission fault/capacity/operation conflict");
      for (const auto &item : goals_)
        require(!item.second.pending, "previous native goal unresolved");
      inspect(end, true);
      deadline(end);
      const auto entered_at = std::to_string(now());
      Entry entry{operation,
                  payload,
                  {{"state", "SEND_ENTERED"}, {"goal_id", invocation}, {"entered_at_ns", entered_at}},
                  nullptr,
                  true,
                  nullptr};
      goals_.emplace(invocation, entry);
      operations_.emplace(operation, invocation);
      recorded_bytes_ += payload.size();
      auto request = std::make_shared<Send::Request>();
      request->goal_id.uuid = goal_id(invocation);
      request->goal = std::move(native);
      try {
        auto response = call<Send>(send_, request, end);
        auto &saved = goals_.at(invocation);
        saved.pending = response->accepted;
        saved.send = {{"state", response->accepted ? "ACCEPTED" : "REJECTED"},
                      {"goal_id", invocation},
                      {"accepted", response->accepted},
                      {"entered_at_ns", entered_at},
                      {"captured_at_ns", std::to_string(now())},
                      {"stamp_sec", response->stamp.sec},
                      {"stamp_nanosec", response->stamp.nanosec}};
        reply(p["sequence"], "SEND_RECORDED", saved.send);
      } catch (const std::exception &e) {
        faulted_ = true;
        goals_.at(invocation).send["state"] = "SEND_UNKNOWN";
        reply(p["sequence"], "SEND_UNKNOWN", goals_.at(invocation).send,
              e.what());
      }
      return;
    }
    if (cmd == "result") {
      keys(body, {"invocation"});
      auto invocation = uuid(body["invocation"]);
      auto known = goals_.find(invocation);
      if (known != goals_.end() && !known->second.result.is_null()) {
        reply(p["sequence"], "RESULT_CAPTURED", known->second.result);
        return;
      }
      auto request = std::make_shared<Result::Request>();
      request->goal_id.uuid = goal_id(invocation);
      const auto query_start = std::to_string(now());
      auto response = call<Result>(result_, request, end);
      require(response->status >= 0 && response->status <= 6,
              "unknown ROS goal status");
      Json value = {{"goal_id", invocation},
                    {"query_started_at_ns", query_start},
                    {"captured_at_ns", std::to_string(now())},
                    {"known_to_bridge", known != goals_.end()},
                    {"ros_goal_status", response->status},
                    {"controller_error_code", response->result.error_code},
                    {"controller_error_string",
                     response->result.error_string.substr(0, 4096)},
                    {"controller_generation_known", false}};
      const bool terminal = response->status == 4 || response->status == 5 ||
                            response->status == 6;
      if (known != goals_.end() && terminal) {
        known->second.result = value;
        known->second.pending = false;
      }
      reply(p["sequence"], terminal ? "RESULT_CAPTURED" : "RESULT_UNKNOWN",
            value);
      return;
    }
    if (cmd == "cancel") {
      keys(body, {"invocation"});
      auto invocation = uuid(body["invocation"]);
      require(goals_.count(invocation) > 0, "cannot cancel an unowned goal");
      auto &saved = goals_.at(invocation);
      if (!saved.cancel.is_null()) {
        reply(p["sequence"], "CANCEL_RECORDED", saved.cancel);
        return;
      }
      const auto entered_at = std::to_string(now());
      saved.cancel = {{"state", "CANCEL_ENTERED"}, {"entered_at_ns", entered_at},
                      {"terminal_stop_proven", false}};
      auto request = std::make_shared<Cancel::Request>();
      request->goal_info.goal_id.uuid =
          goal_id(invocation); // timestamp stays zero: this UUID only, never
                               // cancel-all/before-time.
      try {
        auto response = call<Cancel>(cancel_, request, end);
        Json ids = Json::array();
        for (const auto &target : response->goals_canceling) {
          require(target.goal_id.uuid == goal_id(invocation),
                  "cancel response contains another goal");
          ids.push_back(invocation);
        }
        saved.cancel = {{"state", "RESPONSE"},
                        {"entered_at_ns", entered_at},
                        {"captured_at_ns", std::to_string(now())},
                        {"return_code", response->return_code},
                        {"goals_canceling", ids},
                        {"terminal_stop_proven", false}};
        reply(p["sequence"], "CANCEL_RESPONSE", saved.cancel);
      } catch (const std::exception &e) {
        faulted_ = true;
        saved.cancel["state"] = "CANCEL_UNKNOWN";
        reply(p["sequence"], "CANCEL_UNKNOWN", saved.cancel, e.what());
      }
      return;
    }
    throw std::invalid_argument("unknown bridge command");
  }
};
} // namespace
int main(int argc, char **argv) {
  try {
    require(argc == 2, "usage: rx-ros-jtc-bridge CONFIG");
    Bridge bridge(file(argv[1]));
    bridge.hello();
    std::string line;
    while (packet(line)) {
      Json seq = nullptr;
      try {
        auto p = parse(line);
        if (p.is_object() && p.contains("sequence"))
          seq = p["sequence"];
        bridge.handle(p);
      } catch (const std::runtime_error &e) {
        bridge.reply(seq, "RPC_UNKNOWN", nullptr, e.what());
      } catch (const std::exception &e) {
        bridge.reply(seq, "REJECTED", nullptr, e.what());
      }
    }
    rclcpp::shutdown();
    return 0;
  } catch (const std::exception &e) {
    std::cerr << "rx-ros-jtc-bridge: " << e.what() << '\n';
    if (rclcpp::ok())
      rclcpp::shutdown();
    return 1;
  }
}
